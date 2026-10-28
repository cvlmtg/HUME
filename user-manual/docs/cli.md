# Command-line Flags

HUME's command line is small: you configure almost everything in `init.scm`, not with flags. This page covers the `hume` command run from a shell. For the in-editor `:` prompt, see [Command mode](command-mode.md).

## Synopsis

```sh
hume [OPTIONS] [FILE...]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `FILE...` | One or more files to open, each optionally suffixed with `:LINE` or `:LINE:COLUMN` to place the cursor there on open, e.g. `src/main.rs:42:5`. Both numbers are 1-based, matching the `line:col` the statusline shows. With no arguments, HUME opens a scratch buffer named `*scratch*`. |

A path that exists on disk as typed always opens as-is, so a file named `notes:2024` is never split. Only when the literal path doesn't exist does HUME peel off a trailing position: a nonexistent `foo.rs:42` opens `foo.rs` at line 42 (a lone trailing colon, `foo.rs:42:`, is tolerated too). You can safely paste a `file:line:col` diagnostic from a compiler, linter, or `grep` straight onto the command line. A `0` in either position is a startup error. Both numbers are 1-based, so add 1 to each number of a 0-based diagnostic first.

The position applies only to the pane HUME opens with. Switching to that file from a pane created later (e.g. with `:split`) starts at the top of the file.

## Options

| Flag | Description |
|------|-------------|
| `--keys <STREAM>` | Headless golf-replay mode. Replays the key `STREAM` (e.g. `dwx`) against the input file, laid out on an 80×25 screen, and writes the result to `--output`. Requires `--output` and a single `FILE`, which can carry `:LINE` or `:LINE:COLUMN` like any other. Loads your `init.scm` and plugins like interactive mode; pair with `--no-config` for a pristine, script-independent run. Prints warnings and errors on stderr, and any error makes `hume` exit with status 1. If your config or a plugin fails to load, no keys are replayed and `--output` is not written. |
| `--output <PATH>` | Output path for headless mode. Required by and requires `--keys`. A relative `PATH` is resolved against the shell's working directory once, at startup, so a `:cd` in the key stream does not move it. |
| `--config <FILE>` | Load configuration from `FILE` instead of the default `init.scm`. Themes and the data directory still resolve from the standard directories (see [Configuration](configuration.md#file-locations)). `FILE` must exist and be readable, or it's a startup error. A relative `FILE` is resolved against the shell's working directory once, at startup, and `:reload-config` re-reads that same path, even after a `:cd` in the editor. Not valid with `--no-config`. |
| `--no-config` | Skip `init.scm`: no plugins, no keybindings or settings from your config. Bundled language detection and syntax highlighting still work. Useful to rule out your config while tracking down a bug. `:reload-config` refuses to run for the rest of the session. Not valid with `--config`. |
| `-h`, `--help` | Print help and exit. |
| `-V`, `--version` | Print the HUME version (e.g. `hume x.y.z-f460770`) and exit. The same string is available inside the editor via `:version`. |

## What's not here

HUME has no flags for overriding the log level, theme, or tutor. The `HUME_RUNTIME` environment variable redirects the runtime directory. Logging is in-memory only; read it with `:messages` (see [Files & Buffers](files-and-buffers.md#persistence-and-safety)).

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
