#!/usr/bin/env bash
# Stage an archive-ready directory tree under dist/stage/<name>/.
# Usage: stage.sh <target-triple> <layout>
#   layout = "unix"    -> bin/hume + share/hume/<runtime contents>
#   layout = "windows" -> hume.exe + runtime/<runtime contents>
set -euo pipefail

target="$1"
layout="$2"

root="$(git rev-parse --show-toplevel)"
version="$(cargo metadata --format-version 1 --no-deps \
    --manifest-path "$root/editor/Cargo.toml" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')"
sha="$(git rev-parse --short HEAD)"
name="hume-${version}-${sha}-${target}"
stage="$root/dist/stage/$name"

exe="hume"
[[ "$target" == *windows* ]] && exe="hume.exe"
bin_src="$root/target/$target/release/$exe"

mkdir -p "$root/dist/stage"
rm -rf "$stage"

if [[ "$layout" == "unix" ]]; then
    mkdir -p "$stage/bin" "$stage/share/hume"
    cp "$bin_src" "$stage/bin/hume"
    chmod 755 "$stage/bin/hume"
    cp -R "$root/runtime/." "$stage/share/hume/"
else
    mkdir -p "$stage/runtime"
    cp "$bin_src" "$stage/$exe"
    cp -R "$root/runtime/." "$stage/runtime/"
fi

cp "$root/README.md" "$root/LICENSE" "$stage/"
echo "Staged: $stage"
