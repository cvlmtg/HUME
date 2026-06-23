#!/usr/bin/env bash
# golf.sh — Run HUME against vimgolf-style editing challenges.
#
# Usage: ./golf.sh [challenge-id]
#
# For each challenge directory under tools/golf/challenges the script:
#   1. Copies the challenge `in` file to a temp file.
#   2. Runs `hume --keys "$(cat cmd)" --output <tmp> <tmp>`.
#   3. Compares the result byte-for-byte against `out`.
#   4. Counts keystrokes (each bare char = 1; each <...> token = 1).
#   5. Prints a results table.
#
# Requires Rust toolchain; builds hume-editor from source on each run.

set -euo pipefail

if ! command -v curl &>/dev/null; then
    echo "golf: curl is required for fetching kakoune scores" >&2
    exit 1
fi

CHALLENGES_DIR="$(dirname "$0")/challenges"

challenge_id="${1:-}"

# Build the hume binary unconditionally.
PROJECT_ROOT="$(realpath "$(dirname "$0")/../..")"
echo "golf: building hume-editor ..."
cargo build --package hume-editor --manifest-path "$PROJECT_ROOT/Cargo.toml"
HUME="$PROJECT_ROOT/target/debug/hume"

if [[ -n "$challenge_id" && ! -d "$CHALLENGES_DIR/$challenge_id" ]]; then
    echo "golf: unknown challenge '$challenge_id'" >&2
    exit 1
fi

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

printf "%-28s  %5s  %7s  %s\n" "CHALLENGE" "HUME" "KAKOUNE" "RESULT"
printf "%s\n" "----------------------------------------------------------------------"

if [[ -n "$challenge_id" ]]; then
    dirs=("$CHALLENGES_DIR/$challenge_id")
else
    dirs=("$CHALLENGES_DIR"/*/)
fi

for dir in "${dirs[@]}"; do
    [[ -d "$dir" ]] || continue
    name="$(basename "$dir")"
    in_file="$dir/in"
    out_file="$dir/out"
    cmd_file="$dir/cmd"

    if [[ ! -f "$in_file" || ! -f "$out_file" || ! -f "$cmd_file" ]]; then
        printf "%-28s  %5s  %7s  %s\n" "$name" "-" "-" "SKIP (missing in/out/cmd)"
        continue
    fi

    keys="$(cat "$cmd_file")"
    score="$(count_keys "$keys")"

    kakoune_score="-"
    kakoune_cmd_file="$dir/kakoune_cmd"
    if [[ ! -f "$kakoune_cmd_file" ]]; then
        curl -fsSL --connect-timeout 5 \
            "https://raw.githubusercontent.com/mawww/golf/master/$name/cmd" \
            > "$kakoune_cmd_file" 2>/dev/null || rm -f "$kakoune_cmd_file"
    fi
    if [[ -f "$kakoune_cmd_file" ]]; then
        # Every kakoune golf solution ends with `<space>q`, which is needed to save
        # the result and quit kakoune. HUME doesn't need this, so to make the comparison
        # more fair we subtract 2 from kakoune score.
        kakoune_score="$(( $(count_keys "$(cat "$kakoune_cmd_file")") - 2 ))"
    fi

    tmp="$(mktemp)"
    cp "$in_file" "$tmp"

    if "$HUME" --keys "$keys" --output "$tmp" "$tmp" 2>/dev/null \
       && cmp -s "$out_file" "$tmp"; then
        printf "%-28s  %5s  %7s  %s\n" "$name" "$score" "$kakoune_score" "OK"
        pass=$((pass + 1))
    else
        printf "%-28s  %5s  %7s  %s\n" "$name" "$score" "$kakoune_score" "FAIL"
        if [[ -n "$challenge_id" ]]; then
            echo ""
            git diff --no-index --color "$out_file" "$tmp" \
              | grep -v -E $'^(\033\\[1m|\033\\[36m@@)' || true
        fi
        fail=$((fail + 1))
    fi
    rm -f "$tmp"
done

printf "%s\n" "------------------------------------------------------------"
printf "%d passed, %d failed\n" "$pass" "$fail"

[[ $fail -eq 0 ]]
