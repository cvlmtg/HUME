# Standard Library

`core:stdlib` gives plugin authors small helpers that many plugins need: filesystem, subprocess, selection and config-validation commands. Each one is an editor command reached through `call!`, never a plain Scheme function.

## Setup

```scheme
(load-plugin! "core:stdlib")
```

[Core Plugins](core-plugins.md#core-stdlib) explains why this call should stay a plain `load-plugin!`. [Depending on another plugin](writing-plugins.md#depending-on-another-plugin) shows how to check it's available before your plugin relies on it.

## Selections

| Call | Effect |
|------|--------|
| `stdlib/single-selection?` | `(call! "stdlib/single-selection?" sels)`{.sig} `#t` if `sels` holds exactly one selection |
| `stdlib/all-single-char?` | `(call! "stdlib/all-single-char?" sels)`{.sig} `#t` if every selection in `sels` spans exactly one grapheme |
| `stdlib/cursor-char-offset` | `(call! "stdlib/cursor-char-offset" sels)`{.sig} 0-indexed head char offset of the primary selection in `sels`, or `#f` |
| `stdlib/primary-selection` | `(call! "stdlib/primary-selection" sels)`{.sig} The primary selection in `sels`, or `#f` |

`sels` is the list of selections `(buffer-selections pane)` returns; read one selection with the [`selection-*` calls](plugin-api.md#buffers-panes-selections). All four return `#f` for a `#f` `sels`, so a caller whose `sels` may be `#f` needs no guard at each step.

## Filesystem

| Call | Effect |
|------|--------|
| `stdlib/find` | `(call! "stdlib/find" pred? lst)`{.sig} First element of `lst` satisfying `pred?`, or `#f` |
| `stdlib/concat` | `(call! "stdlib/concat" lists)`{.sig} The lists in `lists` joined into one, in order. Unlike `(apply append lists)`, it stays fast however many lists there are |
| `stdlib/write-file!` | `(call! "stdlib/write-file!" path content)`{.sig} Write `content` to `path`, creating or truncating it |
| `stdlib/delete-dir!` | `(call! "stdlib/delete-dir!" dir)`{.sig} Recursively delete `dir`; idempotent |
| `stdlib/delete-file!` | `(call! "stdlib/delete-file!" path)`{.sig} Delete `path`; idempotent |
| `stdlib/list-entries` | `(call! "stdlib/list-entries" dir)`{.sig} Sorted basenames of every entry of `dir`, files and directories |
| `stdlib/list-subdirs` | `(call! "stdlib/list-subdirs" dir)`{.sig} Sorted basenames of `dir`'s subdirectories |
| `stdlib/path-absolute?` | `(call! "stdlib/path-absolute?" path)`{.sig} Whether `path` names the same place from any working directory: `/x`, and on Windows `C:\x` or `\\server\share` |

`stdlib/delete-dir!` and `stdlib/delete-file!` treat a missing target as no error, unlike `remove-dir!` and the Steel engine's own `delete-file!`. `list-subdirs` skips stray non-directory entries in a directory tree, like `.DS_Store`. `list-entries` and `list-subdirs` raise an error when `dir` doesn't exist; check it with `path-exists?` first when a missing directory means there is nothing to list.

## Downloads

| Call | Effect |
|------|--------|
| `stdlib/download!` | `(call! "stdlib/download!" url path)`{.sig} Download `url` to the file `path` with `curl`. Raises on an HTTP error. `(call! "stdlib/download!" url path offset size)` downloads only `size` bytes starting at byte `offset` |
| `stdlib/verify-sha256!` | `(call! "stdlib/verify-sha256!" path hex)`{.sig} Raise unless the file at `path` has the SHA-256 `hex`, in either case |
| `stdlib/find-extractor` | `(call! "stdlib/find-extractor" format)`{.sig} The program `stdlib/extract-archive!` runs for a `format` archive, `"tar"` or `"zip"`, or `#f` when there is none |
| `stdlib/extract-archive!` | `(call! "stdlib/extract-archive!" format archive dir)`{.sig} Extract `archive`, a `"tar"` or `"zip"` archive, into `dir` (created if missing), and return the extracted regular files. Raises if the archive holds a symlink pointing outside `dir` |

`verify-sha256!` and `extract-archive!` run the platform's own tools: `shasum` on macOS, `sha256sum` on Linux and `certutil` on Windows for the digest, and `tar` or `unzip` for the archive. `tar` reads gzip-compressed archives too. On Windows, a zip archive is extracted with Windows' own `tar`, or with `unzip` or another `tar` on the `PATH` when Windows' is missing.

## Git

| Call | Effect |
|------|--------|
| `stdlib/git-repo?` | `(call! "stdlib/git-repo?")`{.sig} `#t` when the editor's working directory is inside a git work tree |
| `stdlib/git-toplevel` | `(call! "stdlib/git-toplevel")`{.sig} Absolute repo root of the editor's working directory, or `#f` when git is missing or the directory is outside a work tree |

Both answer for HUME's own working directory (`:pwd`), not necessarily the current buffer's. `git-repo?` is `#f` inside a bare repository, even though `git` exits successfully there.

## Command arguments

| Call | Effect |
|------|--------|
| `stdlib/resolve-lang-arg` | `(call! "stdlib/resolve-lang-arg" pane cmd arg)`{.sig} A typed language-name argument, else `pane`'s buffer's language, else `#f` after an info message naming `cmd` |

Use it for a `:` command that takes an optional language name. `arg` is what the user typed after the command, or `#f` if they typed nothing. With no `arg`, it returns the language of the buffer the command was invoked for, the common case. When neither is available, it logs an info message naming `cmd` and returns `#f`, so your command can bail out cleanly.

## Reports

| Call | Effect |
|------|--------|
| `stdlib/report-section` | `(call! "stdlib/report-section" title items hint)`{.sig} One section as text: `title (count)`, then `, hint` when `items` is not empty (`hint` may be `#f`), then one indented line per item, or `none` |
| `stdlib/show-report!` | `(call! "stdlib/show-report!" pane name sections)`{.sig} Shows `sections`, separated by blank lines, on `pane` in the read-only buffer `[name]` |

Use these for a `:` command that lists things, since a list longer than one line does not fit on the status line. A later call with the same `name` replaces that buffer's text. The `:plum-list-*` commands and `:lsp-catalog` use them.

## Word tokenization

| Call | Effect |
|------|--------|
| `stdlib/split-words` | `(call! "stdlib/split-words" pane str)`{.sig} Every word in `str`, tokenized using `pane`'s buffer's own `word-chars` setting |

This is `(split-words str (get-buffer-option pane "word-chars"))`, the classification the `w`/`b` motions and text objects use, so a word here is what one of those would select. Use it when `str` is that buffer's own content, typically one of its lines. Call `split-words` directly for text not tied to a particular buffer, or to classify words differently from that buffer's setting.

## Plugin configuration

| Call | Effect |
|------|--------|
| `stdlib/config-boolean` | `(call! "stdlib/config-boolean" plugin cfg key default)`{.sig} `cfg`'s value for `key`, or `default` if absent; errors if the resolved value isn't `#t`/`#f` |
| `stdlib/config-string` | `(call! "stdlib/config-string" plugin cfg key default)`{.sig} Same, erroring if the resolved value isn't a string |
| `stdlib/config-enum` | `(call! "stdlib/config-enum" plugin cfg key default allowed)`{.sig} Same, erroring if the resolved value isn't one of the symbols in `allowed` |
| `stdlib/config-integer` | `(call! "stdlib/config-integer" plugin cfg key default minimum)`{.sig} Same, erroring if the resolved value isn't an integer, or is below the integer `minimum` |
| `stdlib/config-list` | `(call! "stdlib/config-list" plugin cfg key default)`{.sig} Same, erroring if the resolved value isn't a list of strings |

`cfg` is what `(plugin-config)` returns, and `key` a symbol, like `'signs`. Every error names the calling plugin (`plugin`) and the offending key, so a bad `#:config` value fails at load time naming what to fix. [Configuring a plugin](writing-plugins.md#configuring-a-plugin) covers reading `#:config` in full.

## Picker buffer-placement

| Call | Effect |
|------|--------|
| `stdlib/buffer-actions` | `(call! "stdlib/buffer-actions" handler)`{.sig} A `picker!`/`live-picker!` `#:actions` list binding `Ctrl-o`/`Ctrl-t`/`Ctrl-v`/`Ctrl-s` to `handler` placed in the current pane, a new tab, a vertical split, and a horizontal split respectively |
| `stdlib/with-tab` | `(call! "stdlib/with-tab" handler)`{.sig} Wraps `handler`: opens a new tab, then calls `handler` with the picker's payload |
| `stdlib/with-vsplit` | `(call! "stdlib/with-vsplit" handler)`{.sig} Wraps `handler`: splits the focused pane side by side, then calls `handler` with the picker's payload |
| `stdlib/with-split` | `(call! "stdlib/with-split" handler)`{.sig} Wraps `handler`: splits the focused pane stacked, then calls `handler` with the picker's payload |

`handler` is the same one-argument procedure a picker passes as `on-select` (see [Custom pickers](writing-plugins.md#custom-pickers)'s `#:actions`). Most plugins want `buffer-actions`. `with-tab`/`with-vsplit`/`with-split` are its building blocks, for a custom `#:actions` list with different keys or only some of the four.
