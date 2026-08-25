#!/usr/bin/env bash
# Cut a release: verify the tree, promote CHANGELOG.md's Unreleased section
# to a dated version header, bump hume-editor's Cargo.toml version, commit,
# and tag. Stops short of pushing — .github/workflows/release.yml builds and
# publishes the GitHub release once you push both the commit and the tag.
# Runs on `main` or a maintenance branch (e.g. `0.10.x`) for patch releases —
# see docs/RELEASING.md's "Patching an older release".
# Usage: scripts/release.sh <version>   (e.g. 0.11.0 or v0.11.0)
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

raw_version="${1:?usage: scripts/release.sh <version>  (e.g. 0.11.0 or v0.11.0)}"
version="${raw_version#v}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "error: version must be X.Y.Z, optionally prefixed with 'v', got '$raw_version'" >&2
  exit 1
}

branch="$(git rev-parse --abbrev-ref HEAD)"
[[ "$branch" == "main" || "$branch" =~ ^[0-9]+\.[0-9]+\.x$ ]] || {
  echo "error: must run on main or a maintenance branch (X.Y.x), on $branch" >&2
  exit 1
}

[[ -z "$(git status --porcelain)" ]] || {
  echo "error: working tree not clean" >&2
  exit 1
}

if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
  echo "error: tag v$version already exists" >&2
  exit 1
fi

current="$(cargo metadata --format-version 1 --no-deps \
    --manifest-path hume-editor/Cargo.toml \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next(p["version"] for p in d["packages"] if p["name"] == "hume-editor"))')"
if [[ "$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -1)" != "$version" || "$current" == "$version" ]]; then
  echo "error: $version is not greater than current version $current" >&2
  exit 1
fi

grep -qx '## Unreleased' CHANGELOG.md || {
  echo "error: CHANGELOG.md has no '## Unreleased' header" >&2
  exit 1
}

# Matches CLAUDE.md's mandated pre-push sequence: fmt, then the full suite
# (test-all.sh covers what a bare `cargo test` silently skips). Failing here
# means nothing below ever touches CHANGELOG.md or Cargo.toml.
cargo fmt --all
bash scripts/test-all.sh

release_date="$(date +%Y-%m-%d)"

# Promote Unreleased's entries under a dated version header, same shape as
# every past release bump — the section stays where it is, only gets a name.
perl -0pi -e "s/^## Unreleased\n/## Unreleased\n\n## [$version] - $release_date\n/m" CHANGELOG.md

# Only [package]'s own version line, never a dependency's `version = "..."`.
awk -v ver="$version" '
  /^\[/ { in_pkg = ($0 == "[package]") }
  in_pkg && /^version = / { print "version = \"" ver "\""; next }
  { print }
' hume-editor/Cargo.toml > hume-editor/Cargo.toml.new
mv hume-editor/Cargo.toml.new hume-editor/Cargo.toml

cargo check -p hume-editor --offline >/dev/null

git add CHANGELOG.md hume-editor/Cargo.toml Cargo.lock
git commit -m "release: bump to v$version

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
git tag -a "v$version" -m "v$version"

echo
echo "Tagged v$version locally. Push when ready:"
echo "  git push origin $branch --follow-tags"
