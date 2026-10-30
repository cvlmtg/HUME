# Standard Library

`core:stdlib` gives plugin authors small helpers that many plugins need: selections, lists, files, downloads and archives, git, command arguments, reports, word splitting, config validation and a picker keymap. Each one is an editor command reached through `call!`, never a plain Scheme function.

## Setup

```scheme
(load-plugin! "core:stdlib")
```

[Core Plugins](core-plugins.md#core-stdlib) explains why this call should stay a plain `load-plugin!`. [Depending on another plugin](writing-plugins.md#depending-on-another-plugin) shows how to check it's available before your plugin relies on it.

## Selections

| Call | Effect |
|------|--------|
| `stdlib/single-selection?` | `(call! "stdlib/single-selection?" sels)`{.sig} `#t` if `sels` holds one selection |
| `stdlib/all-single-char?` | `(call! "stdlib/all-single-char?" sels)`{.sig} `#t` if every selection in `sels` covers one character |
| `stdlib/cursor-char-offset` | `(call! "stdlib/cursor-char-offset" sels)`{.sig} The char offset of the primary selection's cursor in `sels`, or `#f` |
| `stdlib/primary-selection` | `(call! "stdlib/primary-selection" sels)`{.sig} The primary selection in `sels`, or `#f` |

`sels` is the list of selections `(buffer-selections pane)` returns; read one selection with the [`selection-*` calls](plugin-api.md#buffers-panes-selections). All four return `#f` when `sels` is `#f`.

## Lists

| Call | Effect |
|------|--------|
| `stdlib/find` | `(call! "stdlib/find" pred? lst)`{.sig} The first element of `lst` satisfying `pred?`, or `#f` |
| `stdlib/concat` | `(call! "stdlib/concat" lists)`{.sig} The lists in `lists` joined into one, in order, however many there are |

## Filesystem

| Call | Effect |
|------|--------|
| `stdlib/write-file!` | `(call! "stdlib/write-file!" path content)`{.sig} Write `content` to `path`, creating or truncating it |
| `stdlib/delete-dir!` | `(call! "stdlib/delete-dir!" dir)`{.sig} Recursively delete `dir`; a missing `dir` is no error |
| `stdlib/delete-file!` | `(call! "stdlib/delete-file!" path)`{.sig} Delete `path`; a missing `path` is no error |
| `stdlib/list-entries` | `(call! "stdlib/list-entries" dir)`{.sig} Sorted basenames of every entry of `dir`, files and directories |
| `stdlib/list-subdirs` | `(call! "stdlib/list-subdirs" dir)`{.sig} Sorted basenames of `dir`'s subdirectories |
| `stdlib/path-absolute?` | `(call! "stdlib/path-absolute?" path)`{.sig} Whether `path` names the same place from any working directory |

Unlike `remove-dir!` and Steel's own `delete-file!`, `stdlib/delete-dir!` and `stdlib/delete-file!` treat a missing target as no error. `list-subdirs` skips stray non-directory entries such as `.DS_Store`. `list-entries` and `list-subdirs` raise when `dir` can't be read, a missing one included; check it with `path-exists?` first when a missing directory means there is nothing to list.

`path-absolute?` is `#t` for a path starting with `/` on macOS and Linux. On Windows it is `#t` for a drive path (`C:\x` or `C:/x`) or a network path (`\\server\share`), and `/x` is not absolute there.

## Downloads

| Call | Effect |
|------|--------|
| `stdlib/download!` | `(call! "stdlib/download!" url path [offset size])`{.sig} Download `url` to the file `path` with `curl`, or only `size` bytes starting at byte `offset`. Raises on an HTTP error |
| `stdlib/verify-sha256!` | `(call! "stdlib/verify-sha256!" path hex)`{.sig} Raise unless the file at `path` has the SHA-256 `hex`, in upper or lower case |
| `stdlib/find-extractor` | `(call! "stdlib/find-extractor" format)`{.sig} The program `stdlib/extract-archive!` runs for `format`, `"tar"` or `"zip"`, or `#f` when none is installed. Raises for any other format |
| `stdlib/extract-archive!` | `(call! "stdlib/extract-archive!" format archive dir)`{.sig} Extract `archive`, a `"tar"` or `"zip"` archive, into `dir` (created if missing), and return the extracted regular files. Raises if the archive holds a symlink pointing outside `dir` |

`stdlib/download!` and `stdlib/extract-archive!` run their program in the terminal, so call them only from a command defined with `#:inline-output #t` (see [Commands](plugin-api.md#commands)); anywhere else they raise. `verify-sha256!` and `find-extractor` work anywhere.

The digest and the archives use the platform's own tools: `shasum` on macOS, `sha256sum` on Linux and `certutil` on Windows, and `tar` or `unzip`. `tar` reads gzip-compressed archives too. On Windows, a zip archive is extracted with Windows' own `tar`, or with `unzip` or another `tar` on the `PATH` when Windows' is missing.

## Git

| Call | Effect |
|------|--------|
| `stdlib/git-repo?` | `(call! "stdlib/git-repo?")`{.sig} `#t` when the editor's working directory is inside a git work tree |
| `stdlib/git-toplevel` | `(call! "stdlib/git-toplevel")`{.sig} The absolute repository root of the editor's working directory, or `#f` when git is missing or the directory is outside a work tree |

Both answer for HUME's own working directory (`:pwd`), not necessarily the current buffer's. `git-repo?` is `#f` inside a bare repository.

## Command arguments

| Call | Effect |
|------|--------|
| `stdlib/resolve-lang-arg` | `(call! "stdlib/resolve-lang-arg" pane cmd arg)`{.sig} A typed language-name argument, else `pane`'s buffer's language, else `#f` after an info message naming `cmd` |

Use it for a typed command that takes an optional language name. `arg` is what the user typed after the command, or `#f` if they typed nothing.

## Reports

| Call | Effect |
|------|--------|
| `stdlib/report-section` | `(call! "stdlib/report-section" title items hint)`{.sig} One section as text: `title (count)`, then `, hint` when `items` is not empty (`hint` may be `#f`), then one indented line per item, or `none` |
| `stdlib/show-report!` | `(call! "stdlib/show-report!" pane name sections)`{.sig} Show `sections`, separated by blank lines, on `pane` in the read-only buffer `[name]` |

Use these for a typed command that lists things, since a list longer than one line does not fit on the statusline. A later call with the same `name` replaces that buffer's text.

## Word tokenization

| Call | Effect |
|------|--------|
| `stdlib/split-words` | `(call! "stdlib/split-words" pane str)`{.sig} Every word in `str`, split by `pane`'s buffer's own `word-chars` setting |

This is `(split-words str (get-buffer-option pane "word-chars"))`, so a word here is what the `w`/`b` motions and word text objects would select in that buffer. Use it when `str` is that buffer's own content, typically one of its lines. Call `split-words` directly for text not tied to a buffer, or to split words differently from that buffer's setting.

## Plugin configuration

| Call | Effect |
|------|--------|
| `stdlib/config-boolean` | `(call! "stdlib/config-boolean" plugin cfg key default)`{.sig} `cfg`'s value for `key`, or `default` if absent; raises if the value isn't `#t`/`#f` |
| `stdlib/config-string` | `(call! "stdlib/config-string" plugin cfg key default)`{.sig} The same, raising if the value isn't a string |
| `stdlib/config-enum` | `(call! "stdlib/config-enum" plugin cfg key default allowed)`{.sig} The same, raising if the value isn't one of the symbols in `allowed` |
| `stdlib/config-integer` | `(call! "stdlib/config-integer" plugin cfg key default minimum)`{.sig} The same, raising if the value isn't an integer, or is below the integer `minimum` |
| `stdlib/config-list` | `(call! "stdlib/config-list" plugin cfg key default)`{.sig} The same, raising if the value isn't a list of strings |

`cfg` is what `(plugin-config)` returns, and `key` a symbol, like `'signs`. Every error names the calling plugin (`plugin`) and the offending key, so a bad `#:config` value fails at load time naming what to fix. [Configuring a plugin](writing-plugins.md#configuring-a-plugin) covers reading `#:config` in full.

## Picker keymap

| Call | Effect |
|------|--------|
| `stdlib/picker-keymap` | `(call! "stdlib/picker-keymap" handler)`{.sig} A keymap for `picker!`/`live-picker!` that calls `handler` with the chosen item; keys below |
| `stdlib/with-tab` | `(call! "stdlib/with-tab" handler)`{.sig} Wrap `handler`: open a new tab, then call `handler` with the picker's payload |
| `stdlib/with-vsplit` | `(call! "stdlib/with-vsplit" handler)`{.sig} Wrap `handler`: split the focused pane side by side, then call `handler` in the new pane with the picker's payload |
| `stdlib/with-split` | `(call! "stdlib/with-split" handler)`{.sig} Wrap `handler`: split the focused pane stacked, then call `handler` in the new pane with the picker's payload |

The keymap's keys:

- `Down`/`Ctrl-n` and `Up`/`Ctrl-p` move the selection; `PageDown`/`PageUp` move it a page, `Ctrl-d`/`Ctrl-u` half a page.
- `Esc` closes the picker.
- `Enter`/`Ctrl-o` close it and call `handler` with the selected item.
- `Ctrl-t`, `Ctrl-v` and `Ctrl-s` do the same in a new tab, a vertical split and a horizontal split.

`handler` takes one argument, the selected item's payload, or `#f` when nothing matches. `picker-keymap` returns a hash, so a plugin changes or adds a key with `hash-insert` (see [Custom pickers](writing-plugins.md#custom-pickers)). `with-tab`/`with-vsplit`/`with-split` are its building blocks, for a binding of your own that opens the item somewhere else. With a `#f` payload they open nothing and still call `handler`. When the focused pane is too small to split, `with-vsplit` and `with-split` leave it as it is and don't call `handler`.
