# Command-line Flags

This page documents the `hume` command as run from a shell. For the in-editor `:` prompt, see [Commands](commands.md).

## Synopsis

```
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
| `-h`, `--help` | Print clap-derived help and exit. |
| `-V`, `--version` | Print the HUME version (e.g. `hume 0.1.0-f460770`) and exit. The same string is available inside the editor via `:version`. |

## What's intentionally not here

HUME has no flags for overriding the config path, log level, theme, or tutor. Configuration lives in `init.scm` (see [Configuration](configuration.md#file-locations)); the runtime directory can be redirected with the `HUME_RUNTIME` environment variable; logging is in-memory only and surfaced via `:messages` (see [Files & Buffers](files-and-buffers.md#persistence-and-safety)).

## Examples

```sh
hume README.md                # open a file
hume src/a.rs src/b.rs        # open multiple files
hume                          # scratch buffer
hume --version
hume --keys 'dwwx' --output out.txt in.txt   # headless replay
```
