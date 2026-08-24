# Releasing

Maintainer checklist. Contributors don't need this — see `CONTRIBUTING.md`.

1. Rename `## Unreleased` in `CHANGELOG.md` to `## [X.Y.Z] - YYYY-MM-DD` and open a fresh
   `## Unreleased` above it. `release.yml` extracts this exact heading shape for the release
   notes, and the publish job fails on a missing section — after all three platform builds have
   already run.
2. Bump `version` in `hume-editor/Cargo.toml`. That is the version users see (`hume --version`,
   the archive names). The library crates are unpublished and stay at `0.1.0`.
3. Commit, then tag and push:

   ```sh
   git tag -a v0.11.0 -m "v0.11.0"
   git push origin main --follow-tags
   ```

Pushing the tag triggers `release.yml` (macOS arm64, Linux x86-64, Windows x86-64 archives, then
the GitHub Release) and rebuilds the manual's release channel from the new tag. The push to `main`
separately triggers the nightly build and the manual's nightly channel.

## Patching an older release

Cut the maintenance branch from the tag only when a patch is actually needed:

```sh
git switch -c 0.10.x v0.10.0
```

Where the fix itself goes depends on whether `main` still has the bug:

- **Both lines have it** — fix on `main` first, then `git cherry-pick -x <sha>` onto `0.10.x`.
  Never the other way around: fixing on the maintenance branch first is how a patch ships in
  `0.10.1` and is silently missing from `0.11.0`. `-x` records the source commit, so the two
  copies stay traceable.
- **Only the old line has it** — a refactor already removed it from `main`, so there is no commit
  to cherry-pick and nothing to fix forward. Commit directly on `0.10.x`. Reproduce on `main`
  before concluding this: a bug that survived the refactor in a different shape is the case above,
  and its two fixes will have diverged too far to cherry-pick either way. Where the regression
  test still compiles against current APIs, add it to `main` regardless — it pins the behaviour
  against a later refactor reintroducing the bug.
- **Embargoed security fix** — maintenance branch first, `main` the same day.

Then follow the steps above on that branch, tagging `v0.10.1`.

### The changelog after a backport

`CHANGELOG.md` documents the project, not the branch, so every release belongs in the copy on
`main` — patches to old lines included. The section travels in the opposite direction to the fix:
the code flows `main` → `0.10.x`, the changelog section flows `0.10.x` → `main`.

Write step 1's section on the maintenance branch — `release.yml` reads the changelog as of the tag
it was triggered by, not as of `main`, so the release notes come out empty otherwise and the
publish job fails. After tagging, copy that section onto `main` as an ordinary doc commit. No
cherry-pick: `main`'s changelog has diverged, and only this one section is wanted.

Place it in **version** order, not release order — `## [0.10.1]` goes *below* `## [0.11.0]` even
though it shipped later. A reader wants the patches for the line they are on grouped with that
line; strict chronological order scatters them. Nothing in the tooling depends on this, since
`release.yml` searches the file for its own version's heading and stops at the next `## [` — an
out-of-sequence section on `main` cannot break a later release.

If HUME ever supports three or more live lines at once, split the file per line the way Node.js
and Kubernetes do (`CHANGELOG/0.10.md`) instead of sorting one file by hand.
