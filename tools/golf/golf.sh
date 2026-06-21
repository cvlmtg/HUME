#!/usr/bin/env bash
# golf.sh — Run HUME against vimgolf-style editing challenges.
#
# Usage: ./golf.sh [CHALLENGES_DIR]
#
# For each challenge directory under CHALLENGES_DIR (default: ./challenges)
# the script:
#   1. Copies the challenge `in` file to a temp file.
#   2. Runs `hume --keys "$(cat cmd)" --output <tmp> <tmp>`.
#   3. Compares the result byte-for-byte against `out`.
#   4. Counts keystrokes (each bare char = 1; each <...> token = 1).
#   5. Prints a results table.
#
# Requires Rust toolchain; builds hume-editor from source on each run.

set -euo pipefail

CHALLENGES_DIR="${1:-$(dirname "$0")/challenges}"

# Build the hume binary unconditionally.
PROJECT_ROOT="$(realpath "$(dirname "$0")/../..")"
echo "golf: building hume-editor ..."
cargo build --package hume-editor --manifest-path "$PROJECT_ROOT/Cargo.toml"
HUME="$PROJECT_ROOT/target/debug/hume"

# Count keystrokes: each bare char = 1; each <...> token = 1.
# Pure-bash implementation — no `expr`, safe with special characters.
count_keys() {
    local keys="$1"
    local count=0
    local i=0
    local len="${#keys}"
    while (( i < len )); do
        if [[ "${keys:$i:1}" == "<" ]]; then
            # Scan forward to the closing '>'; the whole <token> = 1 keystroke.
            local j=$(( i + 1 ))
            while (( j < len )) && [[ "${keys:$j:1}" != ">" ]]; do
                (( j++ )) || true
            done
            (( count++ )) || true
            i=$(( j + 1 ))  # advance past '>'
        else
            (( count++ )) || true
            (( i++ )) || true
        fi
    done
    echo "$count"
}

# ── Run challenges ─────────────────────────────────────────────────────────────

pass=0
fail=0

printf "%-28s  %6s  %s\n" "CHALLENGE" "SCORE" "RESULT"
printf "%s\n" "------------------------------------------------------------"

for dir in "$CHALLENGES_DIR"/*/; do
    [[ -d "$dir" ]] || continue
    name="$(basename "$dir")"
    in_file="$dir/in"
    out_file="$dir/out"
    cmd_file="$dir/cmd"

    if [[ ! -f "$in_file" || ! -f "$out_file" || ! -f "$cmd_file" ]]; then
        printf "%-28s  %6s  %s\n" "$name" "-" "SKIP (missing in/out/cmd)"
        continue
    fi

    keys="$(cat "$cmd_file")"
    score="$(count_keys "$keys")"

    tmp="$(mktemp)"
    cp "$in_file" "$tmp"

    if "$HUME" --keys "$keys" --output "$tmp" "$tmp" 2>/dev/null \
       && cmp -s "$out_file" "$tmp"; then
        printf "%-28s  %6s  %s\n" "$name" "$score" "OK"
        pass=$((pass + 1))
    else
        printf "%-28s  %6s  %s\n" "$name" "$score" "FAIL"
        fail=$((fail + 1))
    fi
    rm -f "$tmp"
done

printf "%s\n" "------------------------------------------------------------"
printf "%d passed, %d failed\n" "$pass" "$fail"

[[ $fail -eq 0 ]]
