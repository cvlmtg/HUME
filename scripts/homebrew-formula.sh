#!/usr/bin/env bash
# Print the Homebrew formula for a release, from the archives it publishes.
# release.yml pushes the output to cvlmtg/homebrew-tap as Formula/hume.rb.
# Usage: homebrew-formula.sh <tag> <dist-dir>
#   (e.g. v0.20.0 dist; every hume-*-<target>.tar.gz in <dist-dir> becomes
#   that platform's url and sha256)
set -euo pipefail

tag="$1"
dist="$2"

# One "<os> <arch> <target>" line per release target the formula serves.
targets=(
  "macos arm aarch64-apple-darwin"
  "linux intel x86_64-unknown-linux-gnu"
  "linux arm aarch64-unknown-linux-gnu"
)

# Prints the on_<arch> blocks of one OS, plus a depends_on when the release
# has an archive for only one of its architectures.
os_block() {
  local want_os="$1" os arch target archive found=() body=""
  for entry in "${targets[@]}"; do
    read -r os arch target <<< "$entry"
    [[ "$os" == "$want_os" ]] || continue
    archive=""
    for path in "$dist"/hume-*-"$target".tar.gz; do
      [[ -e "$path" ]] || continue
      [[ -z "$archive" ]] || {
        echo "error: more than one $target archive in $dist" >&2
        exit 1
      }
      archive="$path"
    done
    [[ -n "$archive" ]] || continue
    found+=("$arch")
    body+="    on_$arch do
      url \"https://github.com/cvlmtg/HUME/releases/download/$tag/$(basename "$archive")\"
      sha256 \"$(shasum -a 256 "$archive" | cut -d' ' -f1)\"
    end
"
  done
  [[ ${#found[@]} -gt 0 ]] || {
    echo "error: no $want_os archive in $dist" >&2
    exit 1
  }
  echo "  on_$want_os do"
  if [[ ${#found[@]} -eq 1 ]]; then
    case "${found[0]}" in
      arm) echo "    depends_on arch: :arm64" ;;
      intel) echo "    depends_on arch: :x86_64" ;;
    esac
    echo
  fi
  printf '%s' "$body"
  echo "  end"
}

# Each block runs in its own subshell, so its `exit 1` only fails the $( );
# capturing first lets `set -e` stop the script before anything is printed.
macos="$(os_block macos)"
linux="$(os_block linux)"

cat <<EOF
class Hume < Formula
  desc "Modal text editor for the terminal"
  homepage "https://cvlmtg.github.io/HUME/"
  license "MIT"

$macos

$linux

  def install
    bin.install "bin/hume"
    pkgshare.install "share/hume/runtime"
  end

  test do
    (testpath/"in.txt").write "hello\\n"
    # :tutor opens the tutor from the runtime directory, and --output writes
    # the buffer it leaves focused.
    system bin/"hume", "--no-config", "--keys", ":tutor<ret>",
           "--output", testpath/"out.txt", testpath/"in.txt"
    assert_match "HUME Tutor", (testpath/"out.txt").read
  end
end
EOF
