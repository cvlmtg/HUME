# HUME demo recording assets

`record.sh` records `demo.gif`, a demo of HUME editing real files of this repository. It runs
HUME in a WezTerm window of its own, types the scenes of `scenes.sh` into it, films that window
with `film.swift` and converts the film to a GIF with `ffmpeg`. HUME starts with
`--config` and WezTerm with `--config-file`, so recording never touches your real
`~/.config/hume` or WezTerm config.

## Layout

- `record.sh` — starts the demo window, plays `scenes.sh`, films it and writes `demo.gif`. It
  defines the commands the scenes are written in: `type_text`, `press`, `pause`,
  `screen_shows`, `wait_screen` and `show`, which starts filming.
- `scenes.sh` — the recording script. It edits `hume-grid/src/geometry.rs` and, in the last
  scene, `user-manual/docs/getting-started.md`. `record.sh` quits HUME with `:qa!` after
  filming stops, so nothing is saved.
- `wezterm.lua` — the demo window's WezTerm config: 110×32 cells of Fira Code, with a title bar
  in the terminal's background colour.
- `film.swift` — films the demo window alone, without the mouse pointer, through macOS's
  ScreenCaptureKit, into a lossless film at 25 frames a second. The window's rounded corners
  come out black, like the title bar.
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

- macOS, with WezTerm installed in `/Applications` and the Fira Code font installed.
- Screen Recording permission (System Settings → Privacy & Security) for the terminal you run
  `record.sh` from.
- The Xcode command-line tools: `record.sh` compiles `film.swift` with `swiftc`.
- A Rust toolchain: `record.sh` runs `cargo build --release` before it opens the window.
- `rust-analyzer` on `$PATH`. It indexes the whole workspace, and the recording waits off-camera
  until the workspace symbol picker lists results.
- The `rust`, `scheme` and `markdown` tree-sitter grammars installed (`:plum-install-grammar
  rust scheme markdown`). A `rust` grammar installed before smart indentation shipped needs
  reinstalling, or the new method in the typing scene is not indented.
- `ffmpeg` and `jq`.

## Re-recording

From the repo root, with `hume-grid/src/geometry.rs` and `user-manual/docs/getting-started.md`
unchanged (`record.sh` refuses to run otherwise, since their changes would show as git signs):

```sh
tools/demo/record.sh
```

The demo window takes focus. Leave the keyboard and mouse alone until the window closes: the
film shows a hollow cursor once another window has focus.

The first scene picks `Rect`'s definition from the workspace symbol picker with fifteen Down
presses, because the picker lists its matches alphabetically and rust-analyzer's fuzzy match
returns other symbols first. A new symbol in the workspace whose name contains the letters of
`Rect` in order changes the match count, and `record.sh` then stops while waiting for the list.
Walk the scenes by hand, recount the row, and update both the count in `rect_matches` and the
Down presses in `scenes.sh`.

To walk the scenes by hand first (recommended before touching timings in `scenes.sh`):

```sh
./target/release/hume --config tools/demo/init.scm hume-grid/src/lib.rs
```

The demo uses your machine's installed tree-sitter grammars (`XDG_DATA_HOME`).
