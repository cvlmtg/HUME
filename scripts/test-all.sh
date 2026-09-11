#!/usr/bin/env bash
# Run the full test suite exactly as CI does (.github/workflows/ci.yml).
#
# Fetches the tree-sitter grammar fixtures the suite needs, then runs every
# test. Run it before pushing.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

bash scripts/fetch-test-grammars.sh

# tools/theme-editor is a separate npm package (pure-logic modules only, no
# JSX under test) with its own test runner. Run before the Rust suite: it takes
# under a second, and a missing Node shouldn't surface only after the slow half
# of the run has already passed.
(cd tools/theme-editor && npm test)

# Denies only `disallowed_methods` — the workspace-wide bans `clippy.toml`
# lists (raw `unicode-width` calls, `std::env::set_var`/`remove_var`, raw
# `ropey` line-index methods). Every other clippy lint stays at its default
# (non-failing) level: adopting those is a separate decision, not a side
# effect of this one.
cargo clippy --workspace --all-targets -- -D clippy::disallowed_methods

# Every crate root denies `rustdoc::broken_intra_doc_links` — this is what
# actually evaluates those links, since the attribute alone is inert without
# a `cargo doc` run to check it against.
cargo doc --workspace --no-deps

cargo test --all-targets
# --all-targets excludes doctests — run them separately so a broken example
# doesn't rot unnoticed.
cargo test --doc
