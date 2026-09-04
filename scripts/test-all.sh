#!/usr/bin/env bash
# Run the full test suite exactly as CI does (.github/workflows/ci.yml).
#
# Fetches the tree-sitter grammar fixtures the suite needs, then runs every
# test. Run it before pushing.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

bash scripts/fetch-test-grammars.sh

cargo test --all-targets --workspace --exclude hume-editor
# hume-editor's tests spin up a Steel engine per test; steel-core 0.8.2's
# mark-and-sweep GC keeps process-global roots (GLOBAL_ROOTS/MARKER in
# steel-core's values/closed.rs), so engines running concurrently on
# separate test threads can race on a weak ref, panicking in
# HeapRef::get()'s unwrap. Serializing this crate's test binary avoids it.
cargo test --all-targets -p hume-editor -- --test-threads=1
# --all-targets excludes doctests — run them separately so a broken example
# doesn't rot unnoticed.
cargo test --doc
