#!/usr/bin/env bash
# Fetch the tree-sitter grammar fixtures the test suite loads.
#
# Usage: scripts/fetch-test-grammars.sh [bundle-dir]
#
# Each fixture is one member of the treesitter.hume release pinned in
# runtime/plugins/core/plum/data/treesitter-hume-pin.scm, for this host's
# platform: the index line names its byte range in the platform's bundle,
# one ranged curl fetches it, and its SHA-256 is checked against that line.
# With a bundle-dir (a `hume-grammar-build build --out` folder), the members
# are read from that folder's bundle instead of the release.
#
# A member unpacks into tests/fixtures/grammars/<name>/ the same way PLUM
# installs it: <name>.<ext>, queries/, manifest.scm and licenses/. Helix's indent
# corpus for a few languages is fetched beside it, at the manifest's
# helix-rev. The fetched members are also packed into a small bundle,
# tests/fixtures/grammars/bundle/, which PLUM's tests install from as a
# local grammar source.
#
# Idempotent: a fixture whose installed digest matches its index line is
# left alone, and the release index is downloaded again only when the pin
# names another tag.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
FIXTURES="$REPO_ROOT/tests/fixtures/grammars"
MEMBERS="$FIXTURES/members"
BUNDLE="$FIXTURES/bundle"
PIN_FILE="$REPO_ROOT/runtime/plugins/core/plum/data/treesitter-hume-pin.scm"
LOCAL_DIR="${1:-}"

TAG="$(grep -v '^;' "$PIN_FILE" | tr -d '"[:space:]')"

# Rust's std::env::consts::OS and ARCH names, which the release uses.
case "$(uname -s)" in
  Darwin)               OS="macos";   EXT="dylib" ;;
  MINGW*|MSYS*|CYGWIN*) OS="windows"; EXT="dll"   ;;
  *)                    OS="linux";   EXT="so"    ;;
esac
case "$(uname -m)" in
  arm64|aarch64) ARCH="aarch64" ;;
  x86_64|amd64)  ARCH="x86_64"  ;;
  *) echo "error: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
PLATFORM="$OS-$ARCH"
RELEASE="https://github.com/cvlmtg/treesitter.hume/releases/download/$TAG"

sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Downloads `url` to `out`, which appears only once the whole file has arrived,
# so an interrupted download is never mistaken for a cached one.
download() {
  local out="$1" url="$2"
  curl -fsSL -o "$out.part" "$url"
  mv "$out.part" "$out"
}

mkdir -p "$FIXTURES" "$MEMBERS"
if [[ -n "$LOCAL_DIR" ]]; then
  INDEX="$LOCAL_DIR/grammars-$PLATFORM.index"
else
  INDEX="$FIXTURES/grammars-$PLATFORM.index"
  cached_tag=""
  [[ -f "$INDEX" ]] && read -r _ cached_tag _ < "$INDEX"
  if [[ "$cached_tag" != "$TAG" ]]; then
    download "$INDEX" "$RELEASE/grammars-$PLATFORM.index"
  fi
fi

read -r magic index_tag index_platform < "$INDEX"
if [[ "$magic" != "treesitter.hume" || "$index_platform" != "$PLATFORM" ]]; then
  echo "error: $INDEX is not a treesitter.hume index for $PLATFORM" >&2
  exit 1
fi
if [[ "$index_tag" != "$TAG" ]]; then
  echo "warning: the bundle is $index_tag, the pin is $TAG" >&2
fi

# Prints "<offset> <size> <sha256>" for `name`, or fails naming it.
index_line() {
  local name="$1" line
  line="$(awk -v n="$name" '$1 == n { print $2, $3, $4 }' "$INDEX")"
  if [[ -z "$line" ]]; then
    echo "error: no $name grammar in treesitter.hume $index_tag for $PLATFORM" >&2
    exit 1
  fi
  echo "$line"
}

# Prints `size` bytes of `file` starting at byte `offset`.
read_range() {
  local file="$1" offset="$2" size="$3"
  head -c "$size" < <(tail -c "+$((offset + 1))" "$file")
}

fetch_grammar() {
  local name="$1" line offset size digest
  line="$(index_line "$name")"
  read -r offset size digest <<< "$line"
  local target="$FIXTURES/$name"
  local member="$MEMBERS/$name.tar.gz"

  if [[ -f "$target/.sha256" && "$(cat "$target/.sha256")" == "$digest" && -f "$member" ]]; then
    echo "  $name: up to date"
    return
  fi

  if [[ -n "$LOCAL_DIR" ]]; then
    read_range "$LOCAL_DIR/grammars-$PLATFORM.tar" "$offset" "$size" > "$member"
  else
    curl -fsSL -r "$offset-$((offset + size - 1))" -o "$member" "$RELEASE/grammars-$PLATFORM.tar"
  fi
  if [[ "$(sha256 "$member")" != "$digest" ]]; then
    rm -f "$member"
    echo "error: $name: the fetched bytes do not match the index digest" >&2
    exit 1
  fi

  rm -rf "$target"
  mkdir -p "$target"
  tar -xzf "$member" -C "$target"
  echo "$digest" > "$target/.sha256"
  echo "  $name: installed"
}

# Fetch Helix's indent test corpus file `file` (tests/indent/<file> at the
# fixture's helix-rev) to helix-indent-corpus.<ext> beside `name`'s grammar.
# Helix's CI checks its indents.scm against these files, so they are the
# expected indentation for the queries the member ships.
fetch_helix_indent_corpus() {
  local name="$1" file="$2"
  local out="$FIXTURES/$name/helix-indent-corpus.${file##*.}"
  if [[ -f "$out" ]]; then
    echo "  $name: $(basename "$out") up to date"
    return
  fi
  local rev
  rev="$(sed -n 's/^ *(helix-rev \. "\([0-9a-f]*\)")*$/\1/p' "$FIXTURES/$name/manifest.scm")"
  if [[ -z "$rev" ]]; then
    echo "error: no helix-rev in $FIXTURES/$name/manifest.scm" >&2
    exit 1
  fi
  download "$out" "https://raw.githubusercontent.com/helix-editor/helix/$rev/tests/indent/$file"
  echo "  $name: fetched $(basename "$out")"
}

# Pack the fetched members into $BUNDLE as an uncompressed ustar archive
# with its index, the same format as a release bundle. Every member name
# fits one 512-byte header, so a member's data starts one block after its
# header and the next header follows its data rounded up to a block. Each
# computed range is read back and checked against the member's digest.
build_bundle() {
  local names=("$@") staging="$FIXTURES/.bundle-staging"
  rm -rf "$staging" "$BUNDLE"
  mkdir -p "$staging" "$BUNDLE"
  local files=()
  for name in "${names[@]}"; do
    cp "$MEMBERS/$name.tar.gz" "$staging/"
    files+=("$name.tar.gz")
  done
  COPYFILE_DISABLE=1 tar --format=ustar -cf "$BUNDLE/grammars-$PLATFORM.tar" -C "$staging" "${files[@]}"

  local index="$BUNDLE/grammars-$PLATFORM.index" pos=0 line size digest check="$staging/check"
  echo "treesitter.hume $TAG $PLATFORM" > "$index"
  for name in "${names[@]}"; do
    line="$(index_line "$name")"
    read -r _ size digest <<< "$line"
    read_range "$BUNDLE/grammars-$PLATFORM.tar" "$((pos + 512))" "$size" > "$check"
    if [[ "$(sha256 "$check")" != "$digest" ]]; then
      echo "error: $name is not at offset $((pos + 512)) of the test bundle" >&2
      exit 1
    fi
    echo "$name $((pos + 512)) $size $digest" >> "$index"
    pos=$((pos + 512 + (size + 511) / 512 * 512))
  done
  rm -rf "$staging"
  echo "  bundle: ${#names[@]} grammars in $BUNDLE"
}

# Byte-wise name order, as a release index lists them.
GRAMMARS=(c html json markdown markdown.inline python rust yaml)

echo "Fetching tree-sitter grammar fixtures ($index_tag, $PLATFORM)..."
for name in "${GRAMMARS[@]}"; do
  fetch_grammar "$name"
done
fetch_helix_indent_corpus "rust" "rust.rs"
fetch_helix_indent_corpus "json" "json.json"
fetch_helix_indent_corpus "python" "python.py"
fetch_helix_indent_corpus "c" "c.c"
build_bundle "${GRAMMARS[@]}"
echo "Done."
