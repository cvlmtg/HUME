#!/usr/bin/env bash
# Fetch and compile tree-sitter grammar fixtures for integration tests.
#
# Idempotent: skips clone if dir already exists, skips compile if the shared
# library is newer than the grammar source. Run once per machine and after
# bumping RUST_REV / JSON_REV.
#
# Compiled artifacts go into tests/fixtures/grammars/ (gitignored).
# Both grammars are ABI 15 (tree-sitter 0.26.x).
set -euo pipefail

RUST_REPO="https://github.com/tree-sitter/tree-sitter-rust"
RUST_REV="77a3747266f4d621d0757825e6b11edcbf991ca5"  # v0.24.2; ABI 15

JSON_REPO="https://github.com/tree-sitter/tree-sitter-json"
JSON_REV="001c28d7a29832b06b0e831ec77845553c89b56d"  # ABI 15

REPO_ROOT="$(git rev-parse --show-toplevel)"
FIXTURES="$REPO_ROOT/tests/fixtures/grammars"
mkdir -p "$FIXTURES"

case "$(uname -s)" in
  Darwin)               EXT="dylib" ;;
  MINGW*|MSYS*|CYGWIN*) EXT="dll"   ;;
  *)                    EXT="so"    ;;
esac

compile_grammar() {
  local name="$1"
  local target="$2"
  local out="$target/parser.$EXT"

  if [[ -f "$out" && "$out" -nt "$target/src/parser.c" ]]; then
    echo "  $name: up to date ($out)"
    return
  fi

  echo "  $name: compiling..."
  tree-sitter build -o "$out" "$target"
  echo "  $name: built $out"
}

fetch_grammar() {
  local name="$1"
  local repo="$2"
  local rev="$3"
  local target="$FIXTURES/$name"

  echo "$name:"
  if [[ ! -d "$target" ]]; then
    echo "  cloning..."
    git clone --filter=blob:none --quiet "$repo" "$target"
    git -C "$target" checkout --quiet --force "$rev"
  else
    local current
    current="$(git -C "$target" rev-parse HEAD)"
    if [[ "$current" != "$rev" ]]; then
      git -C "$target" fetch --quiet
      git -C "$target" checkout --quiet --force "$rev"
    fi
  fi
  compile_grammar "$name" "$target"
}

echo "Fetching tree-sitter grammar fixtures..."
fetch_grammar "rust" "$RUST_REPO" "$RUST_REV"
fetch_grammar "json" "$JSON_REPO" "$JSON_REV"
echo "Done."
