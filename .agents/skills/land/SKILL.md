---
name: land
description: >-
  Land requested iron-canvas changes by merging them locally into the `main`
  branch. Invoke this skill only when the user has explicitly requested a
  landing or merge, not for review, preparation, passing checks, or skill setup.
disable-model-invocation: true
metadata:
  delta-action: land
---

# Land iron-canvas changes locally

Use this workflow only for an explicit request to land changes. The request
itself authorizes carrying out the local merge; do not ask again whether to
merge.

## Preflight and scope

1. Start at the repository root. Confirm the current branch, that local `main`
   exists, and inspect the full status, staged diff, unstaged diff, and
   untracked files. The current local branches are `main` and
   `refactor/grid-core`; this workflow always targets local `main`.
2. Identify exactly which changes belong to the request. Preserve all unrelated
   tracked and untracked work. Do not use `git add -A`, discard changes, stash
   work, reset branches, or switch branches while doing so would overwrite or
   hide unrelated edits. Stop if the intended change set cannot be separated
   safely.
3. If already on `main`, do not attempt to merge `main` into itself. Verify the
   requested changes are committed and tested; otherwise prepare the intended
   commit on `main` and verify it there.
4. If changes are uncommitted on another branch, stage only the reviewed
   intended paths and commit them on that branch with a concise descriptive
   message supplied with `git commit -m "<message>"` so no editor opens. If
   unrelated changes are already staged, stop rather than accidentally
   including them. Do not include unrelated untracked files. If the commit
   identity is missing or the intended paths are unclear, stop and report the
   blocker.

## Verify and merge

1. On the source branch, run `git diff --check` and `cargo test --workspace`.
   The workspace and its member crates are declared in the root
   [`Cargo.toml`](../../../Cargo.toml). Both checks must pass on the candidate
   changes before merging. Do not treat an incomplete or failed test run as
   success.
2. Switch to local `main` only if doing so preserves the existing worktree.
   Merge the source branch with `git merge --no-edit <source-branch>`; use no
   remote, push, pull request, or GitHub operation.
3. Git may merge non-overlapping changes automatically. For conflicts, resolve
   only when the intended result is mechanically clear, such as identical
   edits or an unambiguous whitespace-only conflict. Do not choose `ours` or
   `theirs` wholesale or guess at semantic intent. Pause and report the
   conflicting paths when intent is ambiguous.
4. After the merge, always run `git diff --check` and
   `cargo test --workspace` on the exact merged tree before considering the
   landing complete, including after any conflict resolution. If a check fails,
   leave the local merge state intact for diagnosis and report that it has not
   been verified; do not claim success.

## Confirm the local result

1. Confirm `main` contains the source branch with
   `git merge-base --is-ancestor <source-branch> main`.
2. Inspect `git status --short --branch` and `git log -1 --oneline`. Verify the
   intended changes are on `main` and unrelated local work remains untouched.
   Do not require a clean working tree if unrelated edits were present before
   landing.
3. Keep the source branch and all unrelated local files. Do not publish or
   delete branches as part of this workflow.
