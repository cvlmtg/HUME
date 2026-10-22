#!/usr/bin/env bash
# Fail when an ELF binary requires a newer glibc than <floor>, listing the
# symbols that need it.
# Usage: check-glibc-floor.sh <floor> <binary>...
set -euo pipefail

floor="$1"
shift

# True when version $1 is no newer than version $2.
version_le() {
    [[ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n1)" == "$2" ]]
}

status=0
for bin in "$@"; do
    symbols="$(objdump -T "$bin")"
    if ! versions="$(grep -o 'GLIBC_[0-9][0-9.]*' <<<"$symbols" | cut -d_ -f2 | sort -uV)"; then
        echo "$bin: no versioned glibc symbols found" >&2
        status=1
        continue
    fi
    highest="$(tail -n1 <<<"$versions")"
    if version_le "$highest" "$floor"; then
        echo "$bin: needs glibc $highest (floor $floor)"
        continue
    fi
    echo "$bin: needs glibc $highest, above the floor $floor:" >&2
    while read -r version; do
        if ! version_le "$version" "$floor"; then
            grep -E "GLIBC_${version//./\\.}([^0-9.]|$)" <<<"$symbols" >&2
        fi
    done <<<"$versions"
    status=1
done
exit "$status"
