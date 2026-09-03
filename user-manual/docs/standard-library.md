# Standard Library

`core:stdlib` is a toolkit of small helpers for plugin authors — filesystem, subprocess, selection, and config-validation commands that any plugin might need, so writing one doesn't mean re-deriving them. Every command here is reached through `call!`, never as a plain Scheme function.

## Setup

```scheme
(declare-plugin "core:stdlib")
```

See [Core Plugins](core-plugins.md#core-stdlib) for why this call should stay bare, and [Depending on another plugin](plugins.md#depending-on-another-plugin) for checking it's available before your own plugin relies on it.

## Selections

| Call | Effect |
|------|--------|
| `(call! "stdlib/single-selection?" sels)` | `#t` if `sels` holds exactly one selection |
| `(call! "stdlib/all-single-char?" sels)` | `#t` if every selection in `sels` spans exactly one grapheme |
| `(call! "stdlib/cursor-char-index" sels)` | 0-indexed head char offset of the primary selection in `sels`, or `#f` |
| `(call! "stdlib/primary-selection" sels)` | The primary selection triple in `sels`, or `#f` |
| `(call! "stdlib/selection-anchor" sel)` | Anchor char offset of the selection triple `sel`, or `#f` |
| `(call! "stdlib/selection-head" sel)` | Head char offset of the selection triple `sel`, or `#f` |
| `(call! "stdlib/selection-primary?" sel)` | `#t` if the selection triple `sel` is the primary selection, or `#f` |

`sels` is whatever `(current-selections)` returns — a list of opaque `(anchor head primary?)` triples, char offsets rather than grapheme ordinals. Go through these accessors instead of `car`/`cadr`/`caddr`; all seven accept `#f` and return `#f`, so you only need to check `(current-selections)` for `#f` once, at the call site, rather than inside every helper. `(char-index->line idx)` converts an offset to a line number when you need one.

## Filesystem

| Call | Effect |
|------|--------|
| `(call! "stdlib/find" pred? lst)` | First element of `lst` satisfying `pred?`, or `#f` |
| `(call! "stdlib/write-file" path content)` | Write `content` to `path`, creating or truncating it |
| `(call! "stdlib/delete-dir" dir)` | Recursively delete `dir`; idempotent |
| `(call! "stdlib/delete-file" path)` | Delete `path`; idempotent |
| `(call! "stdlib/list-subdirs" dir)` | Sorted basenames of `dir`'s subdirectories |

`delete-dir` and `delete-file` are idempotent, unlike the Steel scripting engine's own `delete-directory!`/`delete-file!` — a missing target is not an error. `list-subdirs` skips stray non-directory entries that sit alongside a directory tree, like `.DS_Store`.

## Subprocesses

| Call | Effect |
|------|--------|
| `(call! "stdlib/run" cmd args cwd)` | Spawn `cmd`/`args` (in `cwd`, or the inherited directory if `#f`); blocks until exit |

Returns `(stdout stderr exit-code)`. `exit-code` is `#f`, with the failure reason in `stderr`'s place, if the command couldn't even be spawned or its exit couldn't be waited on. `stdlib/run` blocks the whole editor until the command finishes, so it fits something quick (a `git rev-parse`) rather than anything that might take a moment while the user keeps typing — see [Filesystem and processes](plugins.md#filesystem-and-processes) for `run-inline-output!` and `spawn-async!`, the other two ways to run a subprocess.

## Git

| Call | Effect |
|------|--------|
| `(call! "stdlib/git-repo?")` | `#t` when the editor's working directory is inside a git work tree |
| `(call! "stdlib/git-toplevel")` | Absolute repo root of the editor's working directory, or `#f` when git is missing or the directory is outside a work tree |

Both answer for HUME's own working directory (`:pwd`), not necessarily the current buffer's. `git-repo?` is `#f` inside a bare repository, even though `git` itself exits successfully there.

## Command arguments

| Call | Effect |
|------|--------|
| `(call! "stdlib/resolve-lang-arg" cmd arg)` | A typed language-name argument, else the current buffer's language, else `#f` after a warning naming `cmd` |

Use this for a `:` command that takes an optional language name — `arg` is whatever the user typed after the command, or `#f` if they typed nothing. Falling back to the current buffer's language covers the common case of acting on the language you're already looking at; when neither is available, it logs a warning naming `cmd` and returns `#f` so your command can bail out cleanly.

## Plugin configuration

| Call | Effect |
|------|--------|
| `(call! "stdlib/config-boolean" plugin cfg key default)` | `cfg`'s value for `key`, or `default` if absent; errors (naming `plugin`) if the resolved value isn't `#t`/`#f` |
| `(call! "stdlib/config-string" plugin cfg key default)` | Same, erroring if the resolved value isn't a string |
| `(call! "stdlib/config-enum" plugin cfg key default allowed)` | Same, erroring if the resolved value isn't one of the symbols in `allowed` |
| `(call! "stdlib/config-integer" plugin cfg key default minimum)` | Same, erroring if the resolved value isn't an integer, or is below `minimum` (`#f` for no minimum) |
| `(call! "stdlib/config-list" plugin cfg key default)` | Same, erroring if the resolved value isn't a list of strings |

`cfg` is whatever `(plugin-config)` returns. Every error names the calling plugin (`plugin`) and the offending key, so a bad `#:config` value fails at load time pointing at exactly what to fix. See [Configuring a plugin](plugins.md#configuring-a-plugin) for the full picture of reading `#:config`.
