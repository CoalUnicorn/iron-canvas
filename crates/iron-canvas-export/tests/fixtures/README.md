# Formula cache fixtures

These workbooks cover finding 14 in `CRATE_REVIEW.md`.
Each workbook has `Sheet1!A1 = 1+1` and `B1 = label`.
The expected formula result is `2`.
Tests export the original bytes without resaving them.

The missing and stale fixtures come from the local `ux-review-tool` branch,
at `tests/workbook-review/fixtures/issues/`.
Its `PROVENANCE.json` records a locally generated LibreOffice 26.8.0.3 seed.
The seed contains no third-party content.
Only `xl/worksheets/sheet1.xml` was changed to remove or replace the cache.
The numeric variant copies the missing fixture and adds `t="n"` to cell A1.

| File | A1 cache | SHA-256 |
| --- | --- | --- |
| `formula_missing_cache.xlsx` | No value or type | `431b0cc2c3d6098fb2e5e41e80a14d0e5a19b37902eed06ce2981ba9332804ec` |
| `formula_missing_numeric_cache.xlsx` | No value, numeric type | `031038a39c7f9ca0c2c96363160543985ad8be40dcff0150dd855044308b9c71` |
| `formula_stale_cache.xlsx` | Numeric value `99` | `c9d1351aba3e3d4d102caa6b6d1960ca308d72f3204011967c6881944f8a9f99` |
