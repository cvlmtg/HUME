# Contributing to HUME

HUME is a curiosity-driven project. That shapes what contributions fit:

- **Bug reports are always welcome.** Include your OS, terminal emulator, HUME version (`hume --version`), and a minimal config plus the shortest key sequence that reproduces the problem.
- **Small, focused fixes** can go straight to a pull request.
- **Plugins and configuration** are written in Steel, not Rust. If your idea can live in
  `runtime/scheme/`, that is the better place for it.

By contributing you agree that your work is licensed under the [MIT License](LICENSE).

## Getting set up

Prerequisites:

| Need | Why |
|---|---|
| Rust, current stable | The workspace is edition 2024 (needs 1.85 or newer). Install via [rustup](https://rustup.rs). |
| A C compiler | Tree-sitter grammars are compiled from C for the test fixtures. On Windows, MSVC build tools. |
| Node + npm | `tree-sitter-cli` (`npm install -g tree-sitter-cli`) for grammar fixtures; also builds the manual. |
| Bash | The scripts in `scripts/` are bash. On Windows, use Git Bash or WSL. |

Build and run:

```sh
cargo build
cargo run -p hume-editor -- README.md
```

Run from the workspace root — HUME finds its bundled `runtime/` relative to the current
directory in a dev build. From anywhere else, point `HUME_RUNTIME` at `runtime/`.

Orientation: `docs/CRATES.md` maps the ten workspace crates, what each owns, and the dependency edges between them. `docs/ROADMAP.md` tracks open questions. `docs/LEARNING.md` explains the concepts behind the editor.

## Where changes land

`main` is the trunk and the only long-lived branch — branch off it, open the pull request against it. Releases are annotated tags on `main`.

Maintenance branches for older release lines (`0.10.x`) are cut from a tag on demand; don't target one directly. A fix for a bug that also affects an older release still goes to `main` first, and gets cherry-picked back from there after it lands. The exception is a bug that only exists on the older line — where a refactor has already removed it from `main`, there is nothing to fix forward, and the fix is committed on the maintenance branch directly. Open an issue before assuming that is the case.

## Pull requests

- One logical change per pull request. A refactor bundled with a behaviour change is two pull requests.
- Rebase on `main` rather than merging it in. Pull requests are squash-merged, so the final history stays linear and a later cherry-pick is one commit.
- CI runs `cargo fmt --all -- --check` once, then build, the full test suite, and doctests on Linux, macOS, and Windows. All of it must be green.
- Draft pull requests are fine for work in progress; mark them ready when CI is green.

### Verification before you push

In this order, once:

```sh
cargo fmt
scripts/test-all.sh
```

`scripts/test-all.sh` is the whole gate — it reproduces the CI job exactly. It fetches
the tree-sitter grammar fixtures the suite requires before running the tests, so it
needs network access; a bare `cargo test` panics naming the fixture until this has
been run at least once. The first run clones and compiles the grammars into
`tests/fixtures/grammars/` and takes a while; later runs reuse them. The suite also
includes a handful of live network e2e tests (real grammar installs against GitHub),
so a full run always touches the network.

While iterating on one failure, narrow runs are the right tool:

```sh
cargo test -p hume-editor <filter>
```

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org), imperative mood, no trailing period:

```
fix(motion): stop copy-selection from merging into itself on a multi-line selection
refactor(hume-rope): consolidate the display-width helpers
docs(manual): document the trigger-timing rule for keymap plugins
```

Types in use: `feat`, `fix`, `refactor`, `perf`, `docs`, `test`, `style`, `chore`. The scope is optional and is either a crate name (`hume-editor`, `hume-rope`) or a subsystem (`motion`, `decorations`, `lsp`, `scripting`, `runtime`, `render`, `user-manual`).

The subject says what changed; the body says **why**. If the change is not obvious from the diff, the body is not optional.

## Code standards

Read `CLAUDE.md` before your first non-trivial change. Its two invariant sections are the canonical architecture rules for this codebase, not agent-only advice — they cover the selection model, grapheme-cluster boundaries, the ropey-vs-content line-count domains, display-column math, and column naming. Getting one of them wrong produces bugs that are expensive to retrofit out.

Beyond those:

- **Idiomatic Rust.** Pattern matching, iterators, and the type system over runtime checks. `Result` and `Option` — no `.unwrap()` in non-test code, and no `unsafe`.
- **The lints are tests.** `hume-editor/src/editor/lints/` holds source-scanning tests that fail the build on forbidden patterns (raw grapheme stepping, raw line-count derivations, untagged `col` identifiers, direct `unicode-width` calls, and more). If one fires, the pattern it caught is real — fix the code, do not rename around the check.
- **Comments explain why, never what.** Well-named identifiers cover the what. Keep comments self-contained: no references to roadmap files, plan documents, task numbers, or "as discussed".
- **Keep it simple.** This is a learning project as much as a product. Clarity beats cleverness, and a direct solution beats a premature abstraction.
- **Cross-platform.** macOS is primary; Linux and Windows are supported. Platform-specific code lives behind `cfg` gates inside `hume-platform`, nowhere else.

## Tests

Every editing command, text object, and selection operation needs tests. No untested commands.

- **Core editing logic** uses state triples — initial buffer, operation, expected buffer — with cursor and selection markers inline in the text.
- **Appearance** (glyphs, spacing, rendered strings) is tested with [`insta`](https://insta.rs) snapshots: inline snapshots for short one-line elements, file snapshots for full-frame renders.
  Never assert a rendered string by hand in a unit test — unit tests assert data and semantics.

## Changelog

User-visible changes get an entry under `## Unreleased` in `CHANGELOG.md`, written for the person running the editor rather than the person reading the diff. Prefix a breaking change with `**Breaking**:` and say what to change.

## Documentation

Every piece of writing in this repository targets exactly one audience — do not mix them:

| Audience | Lives in |
|---|---|
| End users | `README.md`, `user-manual/docs/`, `runtime/tutor.rst`, `runtime/init.scm.example` |
| Learners | `docs/LEARNING.md`, `docs/learning/` |
| Contributors | Inline source comments, plus a per-directory `README.md` (`scripts/`, `runtime/scheme/`, each `runtime/plugins/core/*/`) |

Source comments target contributors only, and user-facing docs never mention Rust types, module paths, or internal function names.

To preview the manual:

```sh
cd user-manual
npm install
npm run dev
```

The published site has two channels: `nightly` rebuilds from `main` on every merge, and the release channel rebuilds from the newest `v*` tag.

## Security

Please do not open a public issue for a security problem. Use GitHub's **Report a vulnerability** button on the repository's Security tab, which opens a private advisory.
