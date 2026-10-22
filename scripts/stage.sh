#!/usr/bin/env bash
# Stage an archive-ready directory tree under dist/stage/<name>/.
# Usage: stage.sh <target-triple> <layout>
#   layout = "unix"    -> bin/hume + libexec/hume/hume-grammar-build
#                         + share/hume/runtime/<runtime contents>
#   layout = "windows" -> hume.exe + hume-grammar-build.exe
#                         + runtime/<runtime contents>
set -euo pipefail

target="$1"
layout="$2"

root="$(git rev-parse --show-toplevel)"
version="$(cargo metadata --format-version 1 --no-deps \
    --manifest-path "$root/hume-editor/Cargo.toml" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next(p["version"] for p in d["packages"] if p["name"] == "hume-editor"))')"
sha="$(git rev-parse --short HEAD)"
name="hume-${version}-${sha}-${target}"
stage="$root/dist/stage/$name"

exe="hume"
tool="hume-grammar-build"
if [[ "$target" == *windows* ]]; then
    exe="hume.exe"
    tool="hume-grammar-build.exe"
fi
bin_src="$root/target/$target/release/$exe"
tool_src="$root/target/$target/release/$tool"

mkdir -p "$root/dist/stage"
rm -rf "$stage"

if [[ "$layout" == "unix" ]]; then
    mkdir -p "$stage/bin" "$stage/libexec/hume" "$stage/share/hume/runtime"
    cp "$bin_src" "$stage/bin/hume"
    cp "$tool_src" "$stage/libexec/hume/$tool"
    chmod 755 "$stage/bin/hume" "$stage/libexec/hume/$tool"
    cp -R "$root/runtime/." "$stage/share/hume/runtime/"
else
    mkdir -p "$stage/runtime"
    cp "$bin_src" "$stage/$exe"
    cp "$tool_src" "$stage/$tool"
    cp -R "$root/runtime/." "$stage/runtime/"
fi

cp "$root/README.md" "$root/LICENSE" "$stage/"
echo "Staged: $stage"
