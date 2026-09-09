# Releasing

Maintainer checklist. Contributors don't need this — see `CONTRIBUTING.md`.

1. `scripts/release.sh [<version> [<next-version>]]` (e.g. `0.11.0` or `v0.11.0`) on `main` or a maintenance branch (`X.Y.x`), clean tree. Prompts for either version left out (defaulting to the version staged in `Cargo.toml`, and to the next minor on `main` / next patch on a maintenance branch). Runs fmt + `test-all.sh`, promotes the changelog's `## Unreleased` section, bumps `hume-editor`'s version, syncs `Cargo.lock`, commits, tags — then bumps `Cargo.toml` again to the next dev version in a second commit, so `--version` reflects the version in progress rather than the one just tagged.
2. `git push origin <branch> --follow-tags` — carries both commits and the tag, and triggers `release.yml` (three platform archives, then the GitHub Release) and, on `main`, the nightly build.

## Patching an older release

```sh
git switch -c 0.10.x v0.10.0   # only when a patch is actually needed
```

Fix location depends on whether `main` still has the bug:

- **Both lines have it** — fix on `main` first, `git cherry-pick -x <sha>` onto `0.10.x`. Never the reverse: that's how a patch ships in `0.10.1` and is silently missing from `0.11.0`.
- **Only the old line has it** — commit directly on `0.10.x`. Confirm on `main` first: a bug that survived a refactor in a different shape is the case above, not this one.
- **Embargoed security fix** — maintenance branch first, `main` the same day.

Then steps 1–2 above, on `0.10.x`.

### Changelog after a backport

Every release's section lives on `main`, patches included. Write it on the maintenance branch (`release.yml` reads the changelog as of the tag it was triggered by), then copy it onto `main` after tagging as an ordinary doc commit — no cherry-pick, `main`'s changelog has diverged.

Order by **version**, not chronology: `## [0.10.1]` sits below `## [0.11.0]` even though it shipped later, so patches stay grouped with their line. `release.yml` matches its own heading and stops at the next `## [`, so this doesn't affect tooling.

Three or more live lines at once → split per line (`CHANGELOG/0.10.md`, Node/Kubernetes-style) instead of hand-sorting one file.
