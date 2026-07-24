#!/usr/bin/env bash
# Records tools/demo/demo.gif from demo.tape, drops the blank frames VHS can
# capture at mode changes, and writes the GIF with an infinite loop.
# Run from anywhere; see README.md for the dependencies.
set -euo pipefail

# Height of the window bar VHS draws. Its dots are in every frame, so a blank
# terminal is only visible below it.
TITLEBAR_PX=40
# Luma above which a pixel is text rather than background.
TEXT_LUMA=80
# A frame is blank when its mean after thresholding to 0/255 stays below this,
# i.e. under about 0.2% of the terminal area is text. A lone cursor measures
# under 0.1, the emptiest real frame about 2.
BLANK_MEAN_LUMA=0.5

for tool in vhs ffmpeg; do
  command -v "$tool" >/dev/null || { echo "record.sh: $tool not found on PATH" >&2; exit 1; }
done

cd "$(dirname "$0")/../.."

# The recording edits these two files and never saves, but uncommitted changes
# in them would show as git-diff signs from the first frame.
if ! git diff --quiet HEAD -- hume-grid/src/geometry.rs user-manual/docs/getting-started.md; then
  echo "record.sh: commit or stash your changes to hume-grid/src/geometry.rs and user-manual/docs/getting-started.md first" >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

vhs -o "$work/raw.gif" tools/demo/demo.tape

crop="crop=iw:ih-$TITLEBAR_PX:0:$TITLEBAR_PX"
ffmpeg -v error -i "$work/raw.gif" \
  -vf "$crop,lutyuv=y='if(gt(val,$TEXT_LUMA),255,0)',signalstats,metadata=mode=print:key=lavfi.signalstats.YAVG:file=$work/stats.txt" -f null -

blank=$(awk -v max_mean="$BLANK_MEAN_LUMA" '
  /^frame:/ { split($1, f, ":"); n = f[2] }
  /signalstats.YAVG=/ { split($0, v, "="); if (v[2] < max_mean) print n }
' "$work/stats.txt")

keep="1"
if [ -n "$blank" ]; then
  keep="not($(sed 's/.*/eq(n\\,&)/' <<<"$blank" | paste -sd+ -))"
fi
echo "record.sh: dropping $(grep -c . <<<"$blank" || true) blank frame(s)" >&2

ffmpeg -v error -y -i "$work/raw.gif" -fps_mode passthrough \
  -filter_complex "select='$keep',split[a][b];[a]palettegen[p];[b][p]paletteuse" \
  -loop 0 "$work/clean.gif"

mv "$work/clean.gif" tools/demo/demo.gif
