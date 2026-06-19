#!/usr/bin/env bash
# fetch-challenge.sh — Download a challenge from vimgolf.com.
#
# Usage: ./fetch-challenge.sh <CHALLENGE_ID> [CHALLENGES_DIR]
#
# Fetches the challenge JSON from vimgolf.com, writes `in` and `out` files
# to CHALLENGES_DIR/<ID>/ (default: ./challenges/), and creates a blank `cmd`
# file for you to fill in after solving.
#
# Requires: curl, jq
#
# Example:
#   ./fetch-challenge.sh 4d1a34ccce8814b72600002b
#   # then solve it in hume and write the keystrokes to:
#   challenges/4d1a34ccce8814b72600002b/cmd
#   # then score it:
#   ./golf.sh challenges/4d1a34ccce8814b72600002b/

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "usage: $(basename "$0") <CHALLENGE_ID> [CHALLENGES_DIR]" >&2
    exit 1
fi

ID="$1"
CHALLENGES_DIR="${2:-$(dirname "$0")/challenges}"

for tool in curl jq; do
    if ! command -v "$tool" &>/dev/null; then
        echo "fetch-challenge: '$tool' is required but not found." >&2
        exit 1
    fi
done

URL="https://www.vimgolf.com/challenges/${ID}.json"

echo "Fetching $URL ..."
json="$(curl -fsSL "$URL")" || {
    echo "fetch-challenge: failed to fetch $URL" >&2
    exit 1
}

# Extract title (best-effort — field path may vary by vimgolf API version).
title="$(printf '%s' "$json" | jq -r '.data.title // .title // "untitled"')"

# Extract input and output text.
# -j = raw output (no surrounding quotes); sed strips Windows CR and ensures
# a trailing newline so cmp against hume's buffer (always ends with \n) works.
in_data="$(printf '%s' "$json" | jq -j '.in.data' | sed 's/\r$//')"
out_data="$(printf '%s' "$json" | jq -j '.out.data' | sed 's/\r$//')"

if [[ -z "$in_data" || -z "$out_data" ]]; then
    echo "fetch-challenge: could not extract in/out data from $URL" >&2
    echo "  Make sure the challenge ID is correct and the vimgolf API is reachable." >&2
    exit 1
fi

dest="$CHALLENGES_DIR/$ID"
mkdir -p "$dest"

# Ensure both files end with exactly one newline (hume buffer invariant).
printf '%s' "$in_data"  | sed -e '$a\' > "$dest/in"
printf '%s' "$out_data" | sed -e '$a\' > "$dest/out"

# Blank cmd — fill this in after solving the challenge.
if [[ ! -f "$dest/cmd" ]]; then
    printf '' > "$dest/cmd"
fi

echo "Downloaded: $title"
echo "  $dest/"
echo "  Solve it in hume, then write your keystrokes to: $dest/cmd"
echo "  Score it with: ./golf.sh $dest/"
