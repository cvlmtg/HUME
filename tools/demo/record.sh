#!/usr/bin/env bash
# Records tools/demo/demo.gif: runs HUME in its own WezTerm window, plays
# scenes.sh into it, films that window with film.swift and converts the film
# with ffmpeg. Run from anywhere; see README.md for the dependencies.
set -euo pipefail

WEZTERM_APP=/Applications/WezTerm.app
WEZTERM_CLI=$WEZTERM_APP/Contents/MacOS/wezterm
# Pause after each typed character or pressed key, unless a call names its own.
TYPING_MS=90
GIF_FPS=25
GIF_WIDTH=1200

for tool in cargo ffmpeg jq swiftc; do
  command -v "$tool" >/dev/null || { echo "record.sh: $tool not found on PATH" >&2; exit 1; }
done
[ -x "$WEZTERM_CLI" ] || { echo "record.sh: $WEZTERM_CLI not found" >&2; exit 1; }

cd "$(dirname "$0")/../.."

# The recording edits these two files and never saves, but uncommitted changes
# in them would show as git-diff signs from the first frame.
if ! git diff --quiet HEAD -- hume-grid/src/geometry.rs user-manual/docs/getting-started.md; then
  echo "record.sh: commit or stash your changes to hume-grid/src/geometry.rs and user-manual/docs/getting-started.md first" >&2
  exit 1
fi

cargo build --release

work=$(mktemp -d)
gui=
recorder=
# film finishes its file only when it stops on SIGINT.
cleanup() {
  if [ -n "$recorder" ]; then kill -INT "$recorder" 2>/dev/null || true; wait "$recorder" || true; fi
  if [ -n "$gui" ]; then kill "$gui" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT

swiftc -O tools/demo/film.swift -o "$work/film"

pause() {
  sleep "$(printf '%d.%03d' $(($1 / 1000)) $(($1 % 1000)))"
}

send() {
  "$WEZTERM_CLI" cli send-text --pane-id "$pane" --no-paste -- "$1"
}

# type_text TEXT [MS]: types TEXT one character at a time, MS apart.
type_text() {
  local i
  for ((i = 0; i < ${#1}; i++)); do
    send "${1:i:1}"
    pause "${2:-$TYPING_MS}"
  done
}

# press KEY [COUNT]
press() {
  local seq n
  case $1 in
    Enter) seq=$'\r' ;;
    Escape) seq=$'\e' ;;
    Down) seq=$'\e[B' ;;
    Ctrl+D) seq=$'\x04' ;;
    Ctrl+O) seq=$'\x0f' ;;
    Ctrl+S) seq=$'\x13' ;;
    Ctrl+U) seq=$'\x15' ;;
    Ctrl+X) seq=$'\x18' ;;
    *) echo "record.sh: no byte sequence for key $1" >&2; exit 1 ;;
  esac
  for ((n = 0; n < ${2:-1}; n++)); do
    send "$seq"
    pause "$TYPING_MS"
  done
}

# screen_shows REGEX SECONDS: whether the visible screen matches REGEX within
# SECONDS. It checks at least once.
screen_shows() {
  local deadline=$((SECONDS + $2))
  until [[ $("$WEZTERM_CLI" cli get-text --pane-id "$pane") =~ $1 ]]; do
    ((SECONDS < deadline)) || return 1
    sleep 0.25
  done
}

# wait_screen REGEX SECONDS: waits until the visible screen matches REGEX.
wait_screen() {
  screen_shows "$1" "$2" || { echo "record.sh: the screen did not match /$1/ within $2 s" >&2; exit 1; }
}

# Starts filming and returns once the first frame is in. Everything the scenes
# do before it stays off-camera.
show() {
  "$work/film" "$gui" "$work/raw.mov" >"$work/film.out" &
  recorder=$!
  local deadline=$((SECONDS + 10))
  until [ -s "$work/film.out" ]; do
    if ! kill -0 "$recorder" 2>/dev/null; then
      recorder=
      echo "record.sh: film exited before filming" >&2
      exit 1
    fi
    ((SECONDS < deadline)) || { echo "record.sh: film did not start within 10 s" >&2; exit 1; }
    sleep 0.05
  done
}

gui_sockets() {
  compgen -G "$HOME/.local/share/wezterm/gui-sock-*" || true
}

# Launched through open, macOS activates the new window. Started as a child
# of this script, it would open unfocused behind the terminal running it. The
# new instance's process id is the one in the socket it adds.
sockets_before=$(gui_sockets)
open -n -a "$WEZTERM_APP" --args --config-file "$PWD/tools/demo/wezterm.lua" \
  start --always-new-process --cwd "$PWD" \
  -- "$PWD/target/release/hume" --config tools/demo/init.scm hume-grid/src/lib.rs
deadline=$((SECONDS + 10))
until [ -n "$gui" ]; do
  for socket in $(gui_sockets); do
    grep -qxF "$socket" <<<"$sockets_before" || gui=${socket##*/gui-sock-}
  done
  if [ -z "$gui" ]; then
    ((SECONDS < deadline)) || { echo "record.sh: WezTerm opened no socket" >&2; exit 1; }
    sleep 0.1
  fi
done
export WEZTERM_UNIX_SOCKET="$HOME/.local/share/wezterm/gui-sock-$gui"

deadline=$((SECONDS + 10))
until pane=$("$WEZTERM_CLI" cli list --format json | jq -er '.[0].pane_id'); do
  ((SECONDS < deadline)) || { echo "record.sh: WezTerm opened no pane" >&2; exit 1; }
  sleep 0.1
done

source tools/demo/scenes.sh

[ -n "$recorder" ] || { echo "record.sh: scenes.sh never called show" >&2; exit 1; }
kill -INT "$recorder"
wait "$recorder"
recorder=

type_text ":qa!"
press Enter
deadline=$((SECONDS + 10))
while kill -0 "$gui" 2>/dev/null; do
  ((SECONDS < deadline)) || { echo "record.sh: HUME did not quit after :qa!" >&2; exit 1; }
  sleep 0.25
done
gui=

ffmpeg -v error -i "$work/raw.mov" \
  -vf "fps=$GIF_FPS,scale=$GIF_WIDTH:-2:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse" \
  -loop 0 "$work/demo.gif"

mv "$work/demo.gif" tools/demo/demo.gif
