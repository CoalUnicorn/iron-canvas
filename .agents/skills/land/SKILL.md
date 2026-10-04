---
name: land
description: >-
  Land requested iron-canvas changes into `main` in the user's primary Git
  checkout. Invoke this skill only when the user has explicitly requested a
  landing or merge, not for review, preparation, passing checks, or skill setup.
disable-model-invocation: true
metadata:
  delta-action: land
---

# Land iron-canvas changes to primary Git main

Use this workflow only for an explicit request to land changes. The request
itself authorizes carrying out the landing; do not ask again whether to merge.
The target is `main` in the user's primary Git checkout, not merely `main` in
the agent's Delta clone. The `local` remote is the backlink to that checkout;
it is not a publication remote.

## Preflight and scope

1. Start at the repository root. Confirm the current branch, inspect the full
   status, staged diff, unstaged diff, and untracked files, and confirm that
   Delta-clone `main` exists.
2. Inspect configured remotes. Confirm `local` points to the user's primary
   checkout and that its `main` exists. Inspect that checkout's current branch
   and status too; preserve unrelated edits and untracked files there.
3. Identify exactly which changes belong to the request. Do not use
   `git add -A`, discard changes, stash work, reset branches, or switch
   branches while doing so would overwrite or hide unrelated edits. Stop if
   the intended change set cannot be separated safely.
4. If changes are uncommitted, stage only the reviewed intended paths and
   commit them on the source branch with a concise descriptive message
   supplied with `git commit -m "<message>"`. If unrelated changes are already
   staged, stop rather than accidentally including them. Do not include
   unrelated untracked files. If the commit identity is missing or the
   intended change set is unclear, stop and report the blocker.

## Verify and merge

1. On the source branch, run `git diff --check` and `cargo test --workspace`.
   The workspace and its member crates are declared in the root
   [`Cargo.toml`](../../../Cargo.toml). Both checks must pass on the candidate
   changes before merging. Do not treat an incomplete or failed test run as
   success.
2. Fetch `main` from the verified `local` remote and confirm the source commit
   is a fast-forward of `local/main`. Never force-push, update a public/source
   remote, or use `local` for publication.
3. Inspect `git worktree list --porcelain` for the primary Git repository.
   Confirm `main` is not checked out in the primary checkout or another linked
   worktree before updating it through the `local` remote. If it is checked
   out, stop and report that a worktree-safe merge is needed; do not bypass
   Git's checked-out-branch protection.
4. Fast-forward only the primary checkout's `main` to the tested source
   branch with `git push local <source-branch>:refs/heads/main`. Do not switch
   or reset the primary checkout's current branch.
5. Fetch `local/main` again and verify it identifies the landed source commit.
   Run `git diff --check` and `cargo test --workspace` against that exact
   commit in the Delta clone. If either check fails, report that the primary
   branch moved but is not verified; do not claim success.

## Confirm the primary Git result

1. Confirm primary-checkout `main` points at the landed commit and that its
   previous `main` is an ancestor. Inspect its status and current branch to
   verify unrelated work is untouched; do not require a clean working tree if
   unrelated edits were present before landing.
2. Keep the source branch and all unrelated local files. Do not delete
   branches as part of this workflow.
