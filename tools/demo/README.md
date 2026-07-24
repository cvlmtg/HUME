# HUME demo recording assets

`demo.tape` is a [VHS](https://github.com/charmbracelet/vhs) script that records `demo.gif`, a
demo of HUME editing real files of this repository. HUME starts with `--config`, so
recording never touches your real `~/.config/hume`.

## Layout

- `record.sh` — records `demo.tape` and writes `demo.gif`.
- `demo.tape` — the recording script. It edits `hume-grid/src/geometry.rs` and, in the last
  scene, `user-manual/docs/getting-started.md`, then quits with `:qa!` without saving.
- `init.scm` — the demo's own config: the core plugins the scenes use, rust-analyzer
  registered from `$PATH`, the git branch in the statusline, and `demo.scm` declared as a lazy
  local file that loads on the first `:demo-table`.
- `demo.scm` — the plugin the last scene opens through the file picker. `:demo-table` aligns
  a markdown pipe table's columns, redraws its separator row to match and frames the table
  with a separator rule above and below.

The demo runs no language server on `demo.scm`. With no `cog.scm` above it,
`core:steel-server` would start `steel-language-server` with the repo root as its project
root, and that server has crashed on this repository's Scheme files.

## Dependencies

- A release build: `cargo build --release` from the repo root.
- `rust-analyzer` on `$PATH`. It indexes the whole workspace, and the recording waits off-camera
  until the workspace symbol picker lists results.
- The `rust`, `scheme` and `markdown` tree-sitter grammars installed (`:plum-install-grammar
  rust scheme markdown`). A `rust` grammar installed before smart indentation shipped needs
  reinstalling, or the new method in the typing scene is not indented.
- `vhs` and `ffmpeg`.

## Re-recording

From the repo root, with `hume-grid/src/geometry.rs` and `user-manual/docs/getting-started.md`
unchanged (`record.sh` refuses to run otherwise, since their changes would show as git signs):

```sh
tools/demo/record.sh
```

It records with VHS, drops the blank frames (see below), and writes `demo.gif`.

The first scene picks `Rect`'s definition from the workspace symbol picker with seven Down
presses, because the picker lists its matches alphabetically and rust-analyzer's fuzzy match
returns other symbols first. A new symbol in the workspace whose name contains the letters of
`Rect` in order can move that row; recount it from the recording.

To walk the scenes by hand first (recommended before touching timings in the tape):

```sh
./target/release/hume --config tools/demo/init.scm hume-grid/src/lib.rs
```

The demo uses your machine's installed tree-sitter grammars (`XDG_DATA_HOME`).

## Blank frames

A recording can contain a few fully blank frames, each at a mode change. On every mode
change HUME sets the cursor colour (OSC 12) or resets it (OSC 112). Most likely, VHS's
browser terminal (xterm.js) treats that as a theme change and clears the screen before it
repaints, and VHS sometimes captures a frame in between. Which mode changes are hit varies
from take to take, and a real terminal does not show it. `record.sh` removes every frame
whose terminal area is a single flat colour, and the previous frame is held for its duration.
