#!/usr/bin/env bash
# Run the full test suite exactly as CI does (.github/workflows/ci.yml).
#
# A plain `cargo test` skips the live-grammar e2e tests (gated by
# HUME_REQUIRE_LIVE_GRAMMAR_E2E), silently passes grammar-fixture-gated tests
# when fixtures are missing (gated by HUME_REQUIRE_GRAMMAR_FIXTURES), and
# doesn't fetch fixtures at all — those only ran in CI, so UI-affecting
# changes could pass locally and still break there. This script closes that
# gap: run it before pushing.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

bash scripts/fetch-test-grammars.sh

export HUME_REQUIRE_LIVE_GRAMMAR_E2E=1
export HUME_REQUIRE_GRAMMAR_FIXTURES=1
cargo test --all-targets
# --all-targets excludes doctests — run them separately so a broken example
# doesn't rot unnoticed.
cargo test --doc
