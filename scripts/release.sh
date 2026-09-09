#!/usr/bin/env bash
# Cut a release: verify the tree, promote CHANGELOG.md's Unreleased section
# to a dated version header, bump hume-editor's Cargo.toml version, commit,
# and tag — then bump Cargo.toml again to the next dev version in a second
# commit, so `hume --version` on the branch reflects the version in progress
# rather than the one just tagged. Stops short of pushing — one
# `--follow-tags` push carries both commits and the tag; .github/workflows/
# release.yml builds and publishes the GitHub release from the tag.
# Runs on `main` or a maintenance branch (e.g. `0.10.x`) for patch releases —
# see docs/RELEASING.md's "Patching an older release".
# Usage: scripts/release.sh [<version> [<next-version>]]
#   (e.g. 0.11.0 or v0.11.0; prompts for either argument left out)
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

valid_version() { [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; }

# main gets the next minor; a maintenance branch gets the next patch, so a
# patch release's dev version stays on the line the branch exists to serve.
bump_version() {
  local major minor patch
  IFS=. read -r major minor patch <<< "$1"
  if [[ "$branch" == "main" ]]; then
    echo "$major.$((minor + 1)).0"
  else
    echo "$major.$minor.$((patch + 1))"
  fi
}

# Writes to the global `resolved` rather than echoing into $( ): a $( )
# subshell would turn the validation `exit 1` into a subshell exit that only
# stops the script by way of `set -e`, and would need < /dev/tty to prompt.
resolved=""
# $1 = argument (may be empty), $2 = label, $3 = default
resolve_version() {
  local arg="${1#v}" label="$2" default="$3" reply
  if [[ -n "$arg" ]]; then
    valid_version "$arg" || {
      echo "error: $label must be X.Y.Z, optionally prefixed with 'v', got '$1'" >&2
      exit 1
    }
    resolved="$arg"
    return
  fi
  [[ -t 0 ]] || {
    echo "error: no $label given and stdin is not a terminal" >&2
    exit 1
  }
  while :; do
    read -r -p "$label [$default]: " reply || reply=""  # set -e: EOF must not kill the script
    reply="${reply:-$default}"
    reply="${reply#v}"
    valid_version "$reply" && break
    echo "must be X.Y.Z, optionally prefixed with 'v'" >&2
  done
  resolved="$reply"
}

# Only [package]'s own version line, never a dependency's `version = "..."`.
set_cargo_version() {
  awk -v ver="$1" '
    /^\[/ { in_pkg = ($0 == "[package]") }
    in_pkg && /^version = / { print "version = \"" ver "\""; next }
    { print }
  ' hume-editor/Cargo.toml > hume-editor/Cargo.toml.new
  mv hume-editor/Cargo.toml.new hume-editor/Cargo.toml
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

grep -qx '## Unreleased' CHANGELOG.md || {
  echo "error: CHANGELOG.md has no '## Unreleased' header" >&2
  exit 1
}

current="$(cargo metadata --format-version 1 --no-deps \
    --manifest-path hume-editor/Cargo.toml \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next(p["version"] for p in d["packages"] if p["name"] == "hume-editor"))')"

# Cargo.toml normally already names the version being released (RELEASING.md
# pre-bumps it right after each release). If that version is already tagged
# the pre-bump was skipped, so offer the next one instead of a default that
# can only fail the tag check below.
if git rev-parse -q --verify "refs/tags/v$current" >/dev/null; then
  default_version="$(bump_version "$current")"
else
  default_version="$current"
fi

resolve_version "${1-}" "version to release" "$default_version"
version="$resolved"
resolve_version "${2-}" "next dev version" "$(bump_version "$version")"
next_version="$resolved"

if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
  echo "error: tag v$version already exists" >&2
  exit 1
fi

# Equality is the normal case: RELEASING.md pre-bumps Cargo.toml to the next
# release's version right after each release, so build.rs can suffix dev
# builds with -<sha> until HEAD sits on the matching tag. Only a downgrade
# (releasing older than what's staged) is a mistake here; a true re-release of
# an already-tagged version is caught separately by the tag check above.
if [[ "$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -1)" != "$version" ]]; then
  echo "error: $version is older than current version $current" >&2
  exit 1
fi

if [[ "$next_version" == "$version" ]] ||
  [[ "$(printf '%s\n%s\n' "$version" "$next_version" | sort -V | tail -1)" != "$next_version" ]]; then
  echo "error: next dev version $next_version must be newer than $version" >&2
  exit 1
fi

echo "Releasing v$version, then bumping the dev version to v$next_version."

# Matches CLAUDE.md's mandated pre-push sequence: fmt, then the full suite
# (test-all.sh covers what a bare `cargo test` silently skips). Failing here
# means nothing below ever touches CHANGELOG.md or Cargo.toml.
cargo fmt --all
bash scripts/test-all.sh

release_date="$(date +%Y-%m-%d)"

# Promote Unreleased's entries under a dated version header, same shape as
# every past release bump — the section stays where it is, only gets a name.
perl -0pi -e "s/^## Unreleased\n/## Unreleased\n\n## [$version] - $release_date\n/m" CHANGELOG.md

set_cargo_version "$version"
cargo check -p hume-editor --offline >/dev/null

git add CHANGELOG.md hume-editor/Cargo.toml Cargo.lock
git commit -m "release: bump to v$version"
git tag -a "v$version" -m "v$version"

set_cargo_version "$next_version"
cargo check -p hume-editor --offline >/dev/null

git add hume-editor/Cargo.toml Cargo.lock
git commit -m "chore(release): bump dev version to v$next_version" \
  -m "Post-v$version-tag dev bump so \`--version\` reflects the version in
progress instead of the last tagged release."

echo
echo "Tagged v$version and bumped the dev version to v$next_version. Push when ready:"
echo "  git push origin $branch --follow-tags"
