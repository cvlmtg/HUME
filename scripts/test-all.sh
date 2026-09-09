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

cargo test --all-targets
# --all-targets excludes doctests — run them separately so a broken example
# doesn't rot unnoticed.
cargo test --doc
