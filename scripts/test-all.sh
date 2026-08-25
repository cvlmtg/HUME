#!/usr/bin/env bash
# Run the full test suite exactly as CI does (.github/workflows/ci.yml).
#
# Fetches the tree-sitter grammar fixtures the suite needs, then runs every
# test. Run it before pushing.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

bash scripts/fetch-test-grammars.sh

cargo test --all-targets
# --all-targets excludes doctests — run them separately so a broken example
# doesn't rot unnoticed.
cargo test --doc
