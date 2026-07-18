#!/usr/bin/env bash
# Run the full test suite exactly as CI does (.github/workflows/ci.yml).
#
# A plain `cargo test` skips the live-grammar e2e tests (gated by
# HUME_REQUIRE_LIVE_GRAMMAR_E2E) and tests that need the compiled grammar
# fixtures — those only ran in CI, so UI-affecting changes could pass locally
# and still break there. This script closes that gap: run it before pushing.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

bash scripts/fetch-test-grammars.sh

export HUME_REQUIRE_LIVE_GRAMMAR_E2E=1
cargo test --all-targets
