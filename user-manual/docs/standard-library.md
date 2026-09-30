# Standard Library

`core:stdlib` is a toolkit of small helpers for plugin authors: filesystem, subprocess, selection, and config-validation commands that any plugin might need, so writing one doesn't mean re-deriving them. Every command here is reached through `call!`, never as a plain Scheme function.

## Setup

```scheme
(declare-plugin! "core:stdlib")
```

See [Core Plugins](core-plugins.md#core-stdlib) for why this call should stay bare, and [Depending on another plugin](plugins.md#depending-on-another-plugin) for checking it's available before your own plugin relies on it.

## Selections

| Call | Effect |
|------|--------|
| `(call! "stdlib/single-selection?" sels)` | `#t` if `sels` holds exactly one selection |
| `(call! "stdlib/all-single-char?" sels)` | `#t` if every selection in `sels` spans exactly one grapheme |
| `(call! "stdlib/cursor-char-index" sels)` | 0-indexed head char offset of the primary selection in `sels`, or `#f` |
| `(call! "stdlib/primary-selection" sels)` | The primary selection in `sels`, or `#f` |
| `(call! "stdlib/selection-anchor" sel)` | Anchor char offset of the selection `sel`, or `#f` |
| `(call! "stdlib/selection-head" sel)` | Head char offset of the selection `sel`, or `#f` |
| `(call! "stdlib/selection-primary?" sel)` | `#t` if the selection `sel` is the primary selection, or `#f` |

`sels` is whatever `(buffer-selections pane)` returns: a list of `(hash 'anchor a 'head h 'start s 'end e 'primary p)`, char offsets rather than grapheme ordinals. `'start` and `'end` bound exactly what the selection covers, `'end` exclusive, whatever the length of its characters. All seven pass a `#f` `sels`/`sel` straight through as `#f`, so a caller that got one from somewhere else with its own "nothing here" case doesn't need its own guard at every step. `(offset->line pane idx)` converts an offset to a line number when you need one.

## Filesystem

| Call | Effect |
|------|--------|
| `(call! "stdlib/find" pred? lst)` | First element of `lst` satisfying `pred?`, or `#f` |
| `(call! "stdlib/write-file!" path content)` | Write `content` to `path`, creating or truncating it |
| `(call! "stdlib/delete-dir!" dir)` | Recursively delete `dir`; idempotent |
| `(call! "stdlib/delete-file!" path)` | Delete `path`; idempotent |
| `(call! "stdlib/list-subdirs" dir)` | Sorted basenames of `dir`'s subdirectories |
| `(call! "stdlib/safe-path-segment?" name)` | `#t` iff `name` is safe to use as a single path component |

`delete-dir` and `delete-file` are idempotent, unlike the Steel scripting engine's own `delete-directory!`/`delete-file!`: a missing target is not an error. `list-subdirs` skips stray non-directory entries that sit alongside a directory tree, like `.DS_Store`. `safe-path-segment?` rejects an empty name, `.`/`..`, and anything containing a path separator, `:`, `"`, or a NUL. Use it before joining a user-typed or downloaded name onto a path.

## Subprocesses

| Call | Effect |
|------|--------|
| `(call! "stdlib/run!" cmd args #:cwd dir)` | Spawn `cmd`/`args` (in `dir`, or the inherited directory if omitted); blocks until exit |

Returns `(hash 'stdout s 'stderr s 'exit code)`. `'exit` is `#f`, with the failure reason in `'stderr`, if the command couldn't even be spawned or its exit couldn't be waited on. `stdlib/run!` blocks the whole editor until the command finishes, so it fits something quick (a `git rev-parse`) rather than anything that might take a moment while the user keeps typing. See [Filesystem and processes](plugins.md#filesystem-and-processes) for `run-inline-output!` and `spawn-async!`, the other two ways to run a subprocess.

## Git

| Call | Effect |
|------|--------|
| `(call! "stdlib/git-repo?")` | `#t` when the editor's working directory is inside a git work tree |
| `(call! "stdlib/git-toplevel")` | Absolute repo root of the editor's working directory, or `#f` when git is missing or the directory is outside a work tree |

Both answer for HUME's own working directory (`:pwd`), not necessarily the current buffer's. `git-repo?` is `#f` inside a bare repository, even though `git` itself exits successfully there.

## Command arguments

| Call | Effect |
|------|--------|
| `(call! "stdlib/resolve-lang-arg" pane cmd arg)` | A typed language-name argument, else `pane`'s buffer's language, else `#f` after a warning naming `cmd` |

Use this for a `:` command that takes an optional language name: `arg` is whatever the user typed after the command, or `#f` if they typed nothing. Falling back to the invoking buffer's language covers the common case of acting on the language of the buffer the command was invoked for; when neither is available, it logs a warning naming `cmd` and returns `#f` so your command can bail out cleanly.

## Word tokenization

| Call | Effect |
|------|--------|
| `(call! "stdlib/split-words" pane str)` | Every word in `str`, tokenized using `pane`'s buffer's own `word-chars` setting |

Same classification `w`/`b` motions and text objects use, so a word here is exactly what one of those would select. This is `(split-words str (get-buffer-option pane "word-chars"))`. Use it whenever `str` is that buffer's own content (typically one of its lines) and you want that buffer's own notion of a word. Call `split-words` directly for text that isn't tied to a particular buffer, or when you have a real reason to classify differently from that buffer's setting.

## Plugin configuration

| Call | Effect |
|------|--------|
| `(call! "stdlib/config-boolean" plugin cfg key default)` | `cfg`'s value for `key`, or `default` if absent; errors (naming `plugin`) if the resolved value isn't `#t`/`#f` |
| `(call! "stdlib/config-string" plugin cfg key default)` | Same, erroring if the resolved value isn't a string |
| `(call! "stdlib/config-enum" plugin cfg key default allowed)` | Same, erroring if the resolved value isn't one of the symbols in `allowed` |
| `(call! "stdlib/config-integer" plugin cfg key default minimum)` | Same, erroring if the resolved value isn't an integer, or is below `minimum` (`#f` for no minimum) |
| `(call! "stdlib/config-list" plugin cfg key default)` | Same, erroring if the resolved value isn't a list of strings |

## Picker buffer-placement

| Call | Effect |
|------|--------|
| `(call! "stdlib/buffer-actions" handler)` | A `picker!`/`live-picker!` `#:actions` list binding `Ctrl-o`/`Ctrl-t`/`Ctrl-v`/`Ctrl-s` to `handler` placed in the current pane, a new tab, a vertical split, and a horizontal split respectively |
| `(call! "stdlib/with-tab" handler)` | Wraps `handler`: opens a new tab, then calls `handler` with the picker's payload |
| `(call! "stdlib/with-vsplit" handler)` | Wraps `handler`: splits the focused pane side by side, then calls `handler` with the picker's payload |
| `(call! "stdlib/with-split" handler)` | Wraps `handler`: splits the focused pane stacked, then calls `handler` with the picker's payload |

`handler` is the same one-argument procedure a picker already passes as `on-select` (see [Custom pickers](plugins.md#custom-pickers)'s `#:actions`). `buffer-actions` is the one plugin authors reach for; `with-tab`/`with-vsplit`/`with-split` are its building blocks, for composing a custom `#:actions` list with different keys or a subset of the four.

`cfg` is whatever `(plugin-config)` returns. Every error names the calling plugin (`plugin`) and the offending key, so a bad `#:config` value fails at load time pointing at exactly what to fix. See [Configuring a plugin](plugins.md#configuring-a-plugin) for the full picture of reading `#:config`.
