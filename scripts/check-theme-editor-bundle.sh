#!/usr/bin/env bash
# tools/theme-editor/index.html is generated output — a vite build of
# tools/theme-editor/src/ — committed because the user manual links to it
# directly as a standalone download (user-manual/docs/configuration.md,
# from-helix.md). Nothing else checks it against src/: `npm test` runs only
# against src/'s pure logic modules, never the bundle. This closes that gap
# with the same regenerate-or-assert shape as
# hume-engine/src/theme/loader/vocabulary.rs's `theme_vocabulary_js_matches_loader`
# and hume-editor/src/editor/tests/scripting_host_globals.rs's
# `hume_globals_scm_matches_generated_host_names`.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT/tools/theme-editor"

# vite/react are dev dependencies `npm test` never needs, so a checkout that
# has only ever run the test suite has no node_modules — install from the
# lockfile rather than failing on a dependency only this script requires.
[ -d node_modules ] || npm ci

if [ "${HUME_WRITE_THEME_EDITOR:-}" = "1" ]; then
  npm run build # writes ../index.html in place (vite.config.js: outDir '..')
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
npm run build -- --outDir "$tmp"
if ! cmp -s "$tmp/index.html" index.html; then
  echo "error: tools/theme-editor/index.html is stale. Regenerate with:" >&2
  echo "  HUME_WRITE_THEME_EDITOR=1 scripts/check-theme-editor-bundle.sh" >&2
  exit 1
fi
