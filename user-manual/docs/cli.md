# Command-line Flags

HUME's command line is deliberately small — almost everything is configured in `init.scm` rather than passed as a flag. This page covers the `hume` command as run from a shell; for the in-editor `:` prompt, see [Commands](commands.md).

## Synopsis

```sh
hume [OPTIONS] [FILE...]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `FILE...` | One or more files to open. With no arguments, HUME opens a scratch buffer named `*scratch*`. |

## Options

| Flag | Description |
|------|-------------|
| `--keys <STREAM>` | Headless golf-replay mode. Replay the key `STREAM` (e.g. `dwx`) against a single input file and write the result to `--output`. Requires `--output` and exactly one `FILE`. |
| `--output <PATH>` | Output path for headless mode. Required by and requires `--keys`. |
| `--config <FILE>` | Load configuration from `FILE` instead of the default `init.scm`. Themes and the data directory still resolve from the standard directories (see [Configuration](configuration.md#file-locations)). `FILE` must exist — a missing or unreadable path is a startup error. Not valid with `--keys`. |
| `-h`, `--help` | Print help and exit. |
| `-V`, `--version` | Print the HUME version (e.g. `hume x.y.z-f460770`) and exit. The same string is available inside the editor via `:version`. |

## What's not here

HUME has no flags for overriding the log level, theme, or tutor. The runtime directory can be redirected with the `HUME_RUNTIME` environment variable; logging is in-memory only and surfaced via `:messages` (see [Files & Buffers](files-and-buffers.md#persistence-and-safety)).

## Examples

```sh
hume README.md                # open a file
hume src/a.rs src/b.rs        # open multiple files
hume                          # scratch buffer
hume --version
hume --keys 'dwwx' --output out.txt in.txt   # headless replay
hume --config ./demo.scm README.md           # load an alternate init.scm
```
