# Command-line Flags

HUME's command line is deliberately small — almost everything is configured in `init.scm` rather than passed as a flag. This page covers the `hume` command as run from a shell; for the in-editor `:` prompt, see [Command mode](command-mode.md).

## Synopsis

```sh
hume [OPTIONS] [FILE...]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `FILE...` | One or more files to open, each optionally suffixed with `:LINE` or `:LINE:COLUMN` to place the cursor there on open — e.g. `src/main.rs:42:5`. Both numbers are 1-based, matching the `line:col` the statusline shows. With no arguments, HUME opens a scratch buffer named `*scratch*`. |

A path that exists on disk exactly as typed always opens as-is — a file genuinely named `notes:2024` is never split. Only when the literal path doesn't exist is a trailing position peeled off, so a nonexistent `foo.rs:42` opens `foo.rs` at line 42 (a lone trailing colon, `foo.rs:42:`, is tolerated too). This makes it safe to paste a `file:line:col` diagnostic from a compiler, linter, or `grep` straight onto the command line. A `0` in either position is a startup error — both are 1-based, so a 0-based diagnostic needs 1 added to each number first.

The position only takes effect in the pane HUME opens with; switching to that file later from a pane created afterward (e.g. with `:split`) starts at the top of the file instead.

## Options

| Flag | Description |
|------|-------------|
| `--keys <STREAM>` | Headless golf-replay mode. Replay the key `STREAM` (e.g. `dwx`) against a single input file and write the result to `--output`. Requires `--output` and exactly one `FILE`. |
| `--output <PATH>` | Output path for headless mode. Required by and requires `--keys`. |
| `--config <FILE>` | Load configuration from `FILE` instead of the default `init.scm`. Themes and the data directory still resolve from the standard directories (see [Configuration](configuration.md#file-locations)). `FILE` must exist and be readable — otherwise it's a startup error. A relative `FILE` is resolved against the shell's working directory once, at startup, and that resolved path is what `:reload-config` re-reads — a later `:cd` in the editor has no effect on it. Not valid with `--keys`. |
| `-h`, `--help` | Print help and exit. |
| `-V`, `--version` | Print the HUME version (e.g. `hume x.y.z-f460770`) and exit. The same string is available inside the editor via `:version`. |

## What's not here

HUME has no flags for overriding the log level, theme, or tutor. The runtime directory can be redirected with the `HUME_RUNTIME` environment variable; logging is in-memory only and surfaced via `:messages` (see [Files & Buffers](files-and-buffers.md#persistence-and-safety)).

## Examples

```sh
hume README.md                # open a file
hume src/a.rs src/b.rs        # open multiple files
hume src/main.rs:42           # open at line 42
hume src/main.rs:42:5         # open at line 42, column 5
hume                          # scratch buffer
hume --version
hume --keys 'dwwx' --output out.txt in.txt   # headless replay
hume --config ./demo.scm README.md           # load an alternate init.scm
```
