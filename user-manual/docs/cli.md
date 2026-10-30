# Command-line Flags

HUME's command line is small: you configure almost everything in `init.scm`, not with flags. This page covers the `hume` command run from a shell. For the in-editor `:` prompt, see [Command mode](command-mode.md).

## Synopsis

```sh
hume [OPTIONS] [FILE...]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `FILE...` | One or more files to open, each optionally suffixed with `:LINE` or `:LINE:COLUMN` to place the cursor there, e.g. `src/main.rs:42:5`. With no arguments, HUME opens a scratch buffer named `*scratch*`. |

Line and column are 1-based, matching the `line:col` the statusline shows, and a `0` in either is a startup error. A path that exists on disk as typed always opens as-is, so a file named `notes:2024` is never split; only a path that doesn't exist has a trailing position taken off. You can paste a `file:line:col` from a compiler, linter or `grep` straight onto the command line. The position applies to the pane HUME starts with.

## Options

| Flag | Description |
|------|-------------|
| `--keys <STREAM>` | Headless mode: replays the key `STREAM` against a single `FILE` on an 80×25 screen, with no terminal, and writes the result to `--output`. See [Headless mode](#headless-mode). |
| `--output <PATH>` | Where headless mode writes the result. Required by and requires `--keys`. A relative `PATH` is resolved against the shell's working directory at startup, so a `:cd` in the key stream does not move it. |
| `--config <FILE>` | Load configuration from `FILE` instead of the default `init.scm`. Themes and the data directory still resolve from the standard directories (see [Configuration](configuration.md#file-locations)). `FILE` must exist and be readable. A relative `FILE` is resolved at startup, and `:reload-config` re-reads that same path, even after a `:cd`. Not valid with `--no-config`. |
| `--no-config` | Skip `init.scm`: no plugins, no keybindings or settings from your config. Language detection and syntax highlighting still work. Useful to rule out your config while tracking down a bug. `:reload-config` refuses to run for the rest of the session. Not valid with `--config`. |
| `-h`, `--help` | Print help and exit. |
| `-V`, `--version` | Print the HUME version (e.g. `hume x.y.z-f460770`) and exit. `:version` shows the same string inside the editor. |

## Headless mode

`--keys` runs HUME without a terminal: it opens `FILE` (which can carry `:LINE` or `:LINE:COLUMN`), types the key stream into it, and writes the resulting text to `--output`. It loads your `init.scm` and plugins as an interactive session does; add `--no-config` for a run that doesn't depend on your config. Warnings and errors go to stderr, and any error makes `hume` exit with status 1. If your config or a plugin fails to load, no keys are replayed and `--output` is not written.

In the key stream every character is one keystroke, a space included. Other keys go in angle brackets: `<esc>`, `<ret>` (or `<enter>`), `<tab>`, `<backspace>`, `<up>`, `<pageup>`, `<f1>` and so on, with modifiers as `<c-x>` (Ctrl), `<a-x>` (Alt) and `<s-tab>` (Shift). `<lt>` types a literal `<`.

## Environment

`HUME_RUNTIME` points HUME at a different runtime directory (see [Configuration](configuration.md#file-locations)). HUME has no flags for the log level or the theme. Logging is in memory only; read it with `:messages` (see [Files & Buffers](files-and-buffers.md#persistence-and-safety)).

## Examples

```sh
hume README.md                # open a file
hume src/a.rs src/b.rs        # open multiple files
hume src/main.rs:42           # open at line 42
hume src/main.rs:42:5         # open at line 42, column 5
hume                          # scratch buffer
hume --version
hume --keys 'dwwx' --output out.txt --no-config in.txt   # headless replay, no config
hume --config ./demo.scm README.md                        # load an alternate init.scm
hume --no-config README.md                                # start with no plugins/keybindings
```
