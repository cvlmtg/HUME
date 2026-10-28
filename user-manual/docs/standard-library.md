# Standard Library

`core:stdlib` is a toolkit of small helpers for plugin authors: filesystem, subprocess, selection, and config-validation commands that any plugin might need, so writing one doesn't mean re-deriving them. Every command here is reached through `call!`, never as a plain Scheme function.

## Setup

```scheme
(load-plugin! "core:stdlib")
```

See [Core Plugins](core-plugins.md#core-stdlib) for why this call should stay a plain `load-plugin!`, and [Depending on another plugin](writing-plugins.md#depending-on-another-plugin) for checking it's available before your own plugin relies on it.

## Selections

| Call | Effect |
|------|--------|
| `stdlib/single-selection?` | `(call! "stdlib/single-selection?" sels)`{.sig} `#t` if `sels` holds exactly one selection |
| `stdlib/all-single-char?` | `(call! "stdlib/all-single-char?" sels)`{.sig} `#t` if every selection in `sels` spans exactly one grapheme |
| `stdlib/cursor-char-offset` | `(call! "stdlib/cursor-char-offset" sels)`{.sig} 0-indexed head char offset of the primary selection in `sels`, or `#f` |
| `stdlib/primary-selection` | `(call! "stdlib/primary-selection" sels)`{.sig} The primary selection in `sels`, or `#f` |

`sels` is whatever `(buffer-selections pane)` returns: a list of selections. Read one selection with the [`selection-*` calls](plugin-api.md#buffers-panes-selections). All four pass a `#f` `sels` straight through as `#f`, so a caller that got one from somewhere else with its own "nothing here" case doesn't need its own guard at every step.

## Filesystem

| Call | Effect |
|------|--------|
| `stdlib/find` | `(call! "stdlib/find" pred? lst)`{.sig} First element of `lst` satisfying `pred?`, or `#f` |
| `stdlib/concat` | `(call! "stdlib/concat" lists)`{.sig} The lists in `lists` joined into one, in order. Unlike `(apply append lists)`, it stays fast however many lists there are |
| `stdlib/write-file!` | `(call! "stdlib/write-file!" path content)`{.sig} Write `content` to `path`, creating or truncating it |
| `stdlib/delete-dir!` | `(call! "stdlib/delete-dir!" dir)`{.sig} Recursively delete `dir`; idempotent |
| `stdlib/delete-file!` | `(call! "stdlib/delete-file!" path)`{.sig} Delete `path`; idempotent |
| `stdlib/list-subdirs` | `(call! "stdlib/list-subdirs" dir)`{.sig} Sorted basenames of `dir`'s subdirectories |

`delete-dir!` and `delete-file!` are idempotent, unlike `remove-dir!` and the Steel engine's own `delete-file!`: a missing target is not an error. `list-subdirs` skips stray non-directory entries that sit alongside a directory tree, like `.DS_Store`.

## Git

| Call | Effect |
|------|--------|
| `stdlib/git-repo?` | `(call! "stdlib/git-repo?")`{.sig} `#t` when the editor's working directory is inside a git work tree |
| `stdlib/git-toplevel` | `(call! "stdlib/git-toplevel")`{.sig} Absolute repo root of the editor's working directory, or `#f` when git is missing or the directory is outside a work tree |

Both answer for HUME's own working directory (`:pwd`), not necessarily the current buffer's. `git-repo?` is `#f` inside a bare repository, even though `git` itself exits successfully there.

## Command arguments

| Call | Effect |
|------|--------|
| `stdlib/resolve-lang-arg` | `(call! "stdlib/resolve-lang-arg" pane cmd arg)`{.sig} A typed language-name argument, else `pane`'s buffer's language, else `#f` after an info message naming `cmd` |

Use this for a `:` command that takes an optional language name: `arg` is whatever the user typed after the command, or `#f` if they typed nothing. Falling back to the invoking buffer's language covers the common case of acting on the language of the buffer the command was invoked for; when neither is available, it logs an info message naming `cmd` and returns `#f` so your command can bail out cleanly.

## Reports

| Call | Effect |
|------|--------|
| `stdlib/report-section` | `(call! "stdlib/report-section" title items hint)`{.sig} One section as text: `title (count)`, then `, hint` when `items` is not empty (`hint` may be `#f`), then one indented line per item, or `none` |
| `stdlib/show-report!` | `(call! "stdlib/show-report!" pane name sections)`{.sig} Shows `sections`, separated by blank lines, on `pane` in the read-only buffer `[name]` |

Use these for a `:` command that lists things: a list longer than one line does not fit on the status line. A later call with the same `name` replaces that buffer's text. The `:plum-list-*` commands and `:lsp-catalog` use them.

## Word tokenization

| Call | Effect |
|------|--------|
| `stdlib/split-words` | `(call! "stdlib/split-words" pane str)`{.sig} Every word in `str`, tokenized using `pane`'s buffer's own `word-chars` setting |

Same classification `w`/`b` motions and text objects use, so a word here is exactly what one of those would select. This is `(split-words str (get-buffer-option pane "word-chars"))`. Use it whenever `str` is that buffer's own content (typically one of its lines) and you want that buffer's own notion of a word. Call `split-words` directly for text that isn't tied to a particular buffer, or when you have a real reason to classify differently from that buffer's setting.

## Plugin configuration

| Call | Effect |
|------|--------|
| `stdlib/config-boolean` | `(call! "stdlib/config-boolean" plugin cfg key default)`{.sig} `cfg`'s value for `key`, or `default` if absent; errors (naming `plugin`) if the resolved value isn't `#t`/`#f` |
| `stdlib/config-string` | `(call! "stdlib/config-string" plugin cfg key default)`{.sig} Same, erroring if the resolved value isn't a string |
| `stdlib/config-enum` | `(call! "stdlib/config-enum" plugin cfg key default allowed)`{.sig} Same, erroring if the resolved value isn't one of the symbols in `allowed` |
| `stdlib/config-integer` | `(call! "stdlib/config-integer" plugin cfg key default minimum)`{.sig} Same, erroring if the resolved value isn't an integer, or is below the integer `minimum` |
| `stdlib/config-list` | `(call! "stdlib/config-list" plugin cfg key default)`{.sig} Same, erroring if the resolved value isn't a list of strings |

`cfg` is whatever `(plugin-config)` returns, and `key` a symbol, like `'signs`. Every error names the calling plugin (`plugin`) and the offending key, so a bad `#:config` value fails at load time naming what to fix. See [Configuring a plugin](writing-plugins.md#configuring-a-plugin) for the full picture of reading `#:config`.

## Picker buffer-placement

| Call | Effect |
|------|--------|
| `stdlib/buffer-actions` | `(call! "stdlib/buffer-actions" handler)`{.sig} A `picker!`/`live-picker!` `#:actions` list binding `Ctrl-o`/`Ctrl-t`/`Ctrl-v`/`Ctrl-s` to `handler` placed in the current pane, a new tab, a vertical split, and a horizontal split respectively |
| `stdlib/with-tab` | `(call! "stdlib/with-tab" handler)`{.sig} Wraps `handler`: opens a new tab, then calls `handler` with the picker's payload |
| `stdlib/with-vsplit` | `(call! "stdlib/with-vsplit" handler)`{.sig} Wraps `handler`: splits the focused pane side by side, then calls `handler` with the picker's payload |
| `stdlib/with-split` | `(call! "stdlib/with-split" handler)`{.sig} Wraps `handler`: splits the focused pane stacked, then calls `handler` with the picker's payload |

`handler` is the same one-argument procedure a picker already passes as `on-select` (see [Custom pickers](writing-plugins.md#custom-pickers)'s `#:actions`). `buffer-actions` is the one plugin authors reach for; `with-tab`/`with-vsplit`/`with-split` are its building blocks, for composing a custom `#:actions` list with different keys or a subset of the four.
