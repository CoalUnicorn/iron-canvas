//! Prepared content shared between sessions. Entries contain no overlays.

use std::collections::VecDeque;

use iron_canvas_ironcalc::PreparedWorksheet;

use crate::RenderRequest;

/// Host identity for a text metric environment and its font generation.
///
/// Equal keys promise equal text measurements and font resolution across all
/// sessions that share a cache. Use separate profiles for different backends
/// unless their measurements are equal. Advance the generation after a font
/// change. Use checked counters; never wrap or saturate either value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextMetricsKey {
    profile: u64,
    generation: u64,
}

impl TextMetricsKey {
    pub const fn new(profile: u64, generation: u64) -> Self {
        Self {
            profile,
            generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CacheKey {
    workbook_id: u64,
    sheet: u32,
}

impl CacheKey {
    fn for_request(request: &RenderRequest) -> Self {
        Self {
            workbook_id: request.revision.workbook_id,
            sheet: request.sheet,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SheetEntry {
    key: CacheKey,
    revision: u64,
    viewport: iron_canvas_ironcalc::WorksheetViewport,
    theme: iron_canvas_core::CanvasTheme,
    metrics_key: TextMetricsKey,
    base: PreparedWorksheet,
}

pub(crate) enum CacheLookup<'a> {
    Content(&'a PreparedWorksheet),
    Geometry(&'a PreparedWorksheet),
    Miss,
}

/// Bounded cache with one prepared viewport per workbook and sheet.
///
/// The least recently used entry leaves first. A lookup does not change the
/// use order. Only a successful frame commit inserts or refreshes an entry.
/// Workbook identities are unique across all sessions that share this cache.
#[derive(Debug)]
pub struct WorksheetCache {
    capacity: usize,
    entries: VecDeque<SheetEntry>,
}

impl WorksheetCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Apply the new limit immediately. Keep the most recently used entries.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        while self.entries.len() > capacity {
            self.entries.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn lookup(
        &self,
        request: &RenderRequest,
        metrics_key: TextMetricsKey,
    ) -> CacheLookup<'_> {
        let key = CacheKey::for_request(request);
        let Some(entry) = self.entries.iter().find(|entry| entry.key == key) else {
            return CacheLookup::Miss;
        };
        if entry.theme != request.theme || entry.metrics_key != metrics_key {
            return CacheLookup::Miss;
        }
        if entry.revision == request.revision.revision && entry.viewport == request.viewport {
            CacheLookup::Content(&entry.base)
        } else {
            // A revision or scroll change can keep equal geometry. The compiler
            // resamples the axes before it accepts this geometry.
            CacheLookup::Geometry(&entry.base)
        }
    }

    pub(crate) fn publish(
        &mut self,
        request: &RenderRequest,
        metrics_key: TextMetricsKey,
        base: PreparedWorksheet,
    ) {
        if self.capacity == 0 {
            return;
        }
        let key = CacheKey::for_request(request);
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            self.entries.remove(index);
        }
        self.entries.push_back(SheetEntry {
            key,
            revision: request.revision.revision,
            viewport: request.viewport,
            theme: request.theme.clone(),
            metrics_key,
            base,
        });
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanvasSession, RenderError, RevisionToken};
    use iron_canvas_core::{
        CanvasTheme, CellCoord,
        painter::TextMetrics,
        scene::{GridScene, RenderBackend},
    };
    use iron_canvas_export::SvgSceneBackend;
    use iron_canvas_ironcalc::{WorksheetCompiler, WorksheetViewport};
    use ironcalc_base::UserModel;
    use std::cell::Cell;

    const METRICS: TextMetricsKey = TextMetricsKey::new(1, 0);

    fn request(sheet: u32) -> RenderRequest {
        RenderRequest {
            sheet,
            revision: RevisionToken {
                workbook_id: 1,
                revision: 0,
            },
            viewport: WorksheetViewport {
                first_cell: CellCoord { row: 1, col: 1 },
                width: 200,
                height: 80,
            },
            theme: CanvasTheme::light(),
            overlays: Default::default(),
        }
    }

    fn base() -> PreparedWorksheet {
        WorksheetCompiler::new()
            .prepare(
                &UserModel::new_empty("cache", "en", "UTC", "en").unwrap(),
                0,
                request(0).viewport,
                SvgSceneBackend::new().metrics(),
                &CanvasTheme::light(),
            )
            .unwrap()
    }

    #[test]
    fn validity_keeps_one_entry_per_sheet_and_separates_workbooks() {
        let mut cache = WorksheetCache::new(3);
        let original = request(0);
        cache.publish(&original, METRICS, base());
        assert!(matches!(
            cache.lookup(&original, METRICS),
            CacheLookup::Content(_)
        ));
        let mut changed = original.clone();
        changed.revision.revision += 1;
        assert!(matches!(
            cache.lookup(&changed, METRICS),
            CacheLookup::Geometry(_)
        ));
        changed = original.clone();
        changed.viewport.first_cell.row += 1;
        assert!(matches!(
            cache.lookup(&changed, METRICS),
            CacheLookup::Geometry(_)
        ));
        cache.publish(&changed, METRICS, base());
        assert_eq!(cache.len(), 1);
        assert!(matches!(
            cache.lookup(&original, METRICS),
            CacheLookup::Geometry(_)
        ));
        changed = original.clone();
        changed.theme = CanvasTheme::dark();
        assert!(matches!(cache.lookup(&changed, METRICS), CacheLookup::Miss));
        for key in [TextMetricsKey::new(2, 0), TextMetricsKey::new(1, 1)] {
            assert!(matches!(cache.lookup(&original, key), CacheLookup::Miss));
        }
        changed = original.clone();
        changed.revision.workbook_id += 1;
        assert!(matches!(cache.lookup(&changed, METRICS), CacheLookup::Miss));
        cache.publish(&changed, METRICS, base());
        assert_eq!(cache.len(), 2);
        assert!(matches!(
            cache.lookup(&request(1), METRICS),
            CacheLookup::Miss
        ));
    }

    #[test]
    fn only_publication_refreshes_use_order_and_capacity_applies_immediately() {
        let mut cache = WorksheetCache::new(2);
        let base = base();
        for sheet in [0, 1] {
            cache.publish(&request(sheet), METRICS, base.clone());
        }
        let before = cache.entries.clone();
        cache.lookup(&request(0), METRICS);
        assert_eq!(cache.entries, before);
        cache.publish(&request(0), METRICS, base.clone());
        cache.publish(&request(2), METRICS, base.clone());
        assert!(matches!(
            cache.lookup(&request(1), METRICS),
            CacheLookup::Miss
        ));
        cache.set_capacity(1);
        assert_eq!(cache.capacity(), 1);
        assert!(matches!(
            cache.lookup(&request(0), METRICS),
            CacheLookup::Miss
        ));
        assert!(matches!(
            cache.lookup(&request(2), METRICS),
            CacheLookup::Content(_)
        ));
        cache.set_capacity(0);
        cache.publish(&request(3), METRICS, base.clone());
        assert!(cache.is_empty());
        cache.set_capacity(2);
        cache.publish(&request(0), METRICS, base);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[derive(Clone, Copy)]
    enum Failure {
        None,
        Stage,
        Commit,
    }

    struct FailingBackend {
        svg: SvgSceneBackend,
        failure: Cell<Failure>,
    }

    impl RenderBackend for FailingBackend {
        type StagedFrame = String;
        type Error = &'static str;
        fn metrics(&self) -> &dyn TextMetrics {
            self.svg.metrics()
        }
        fn stage_frame(&mut self, scene: &GridScene) -> Result<String, Self::Error> {
            if matches!(self.failure.get(), Failure::Stage) {
                return Err("stage");
            }
            Ok(self.svg.stage_frame(scene).unwrap())
        }
        fn commit_frame(&mut self, staged: String) -> Result<(), Self::Error> {
            if matches!(self.failure.get(), Failure::Commit) {
                return Err("commit");
            }
            self.svg.commit_frame(staged).unwrap();
            Ok(())
        }
    }

    #[test]
    fn failed_attempts_preserve_entries_use_order_frame_and_pending_invalidation() {
        let mut model = UserModel::new_empty("failures", "en", "UTC", "en").unwrap();
        model.new_sheet().unwrap();
        let mut session = CanvasSession::new(FailingBackend {
            svg: SvgSceneBackend::new(),
            failure: Cell::new(Failure::None),
        });
        let mut cache = WorksheetCache::new(2);
        for sheet in [0, 1] {
            session
                .render_cached(&mut cache, &model, &request(sheet), METRICS)
                .unwrap();
        }
        let before = cache.entries.clone();
        let frame = session.committed_frame().cloned();
        let document = session.backend().svg.document().unwrap().to_owned();
        // Exercise hits, replacement, and insertion. None can publish on error.
        for failure in [Failure::Stage, Failure::Commit] {
            session.backend().failure.set(failure);
            for change in 0..3 {
                let mut candidate = request(0);
                if change == 1 {
                    candidate.revision.revision += 1;
                }
                if change == 2 {
                    candidate.revision.workbook_id += 1;
                }
                assert!(matches!(
                    session.render_cached(&mut cache, &model, &candidate, METRICS),
                    Err(RenderError::Backend(_))
                ));
                assert_eq!(cache.entries, before);
                assert_eq!(session.committed_frame(), frame.as_ref());
                assert_eq!(session.backend().svg.document(), Some(document.as_str()));
            }
        }
        session.backend().failure.set(Failure::None);
        let mut invalid = request(u32::MAX);
        invalid.viewport.width = 0;
        assert!(matches!(
            session.render_cached(&mut cache, &model, &invalid, METRICS),
            Err(RenderError::Preparation(_))
        ));
        assert_eq!(cache.entries, before);
        assert_eq!(session.committed_frame(), frame.as_ref());

        session.invalidate();
        session.backend().failure.set(Failure::Commit);
        assert!(
            session
                .render_cached(&mut cache, &model, &request(0), METRICS)
                .is_err()
        );
        assert_eq!(cache.entries, before);
        session.backend().failure.set(Failure::None);
        let retry = session
            .render_cached(&mut cache, &model, &request(0), METRICS)
            .unwrap();
        assert_eq!(retry.sequence, 3);
        assert!(!retry.content_cached);
        assert!(retry.extended_style_reads > 0);
        assert_eq!(cache.entries.back().unwrap().key.sheet, 0);
        assert!(
            session
                .render_cached(&mut cache, &model, &request(0), METRICS)
                .unwrap()
                .content_cached
        );
    }
}
