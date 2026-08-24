# core:stdlib

General-purpose standard library for plugin authors — a growing toolkit of commands any
plugin might need, exposed via `call!` so cross-plugin code never has to re-derive them.

## Usage

```scheme
(declare-plugin "core:stdlib")
```

With no explicit `#:commands`/`#:events`/`#:languages`, this reads `core:stdlib`'s own
`manifest.scm`, which declares every command below as an activation trigger — `call!`
dispatch activates an unactivated plugin inline before retrying the call, so a
lazily-declared `core:stdlib` still activates transparently the first time another
plugin's command body calls one of them at runtime.

`(load-plugin "core:stdlib")` also works, loading it eagerly instead. Some dependents
require this — see the Caveat below.

**Caveat**: `core:git-diff`, `core:pickers`, and `core:vim-keybind` all validate their
`#:config` through the commands below, and `core:lsp` lists installed servers through
`stdlib/list-subdirs`, so each checks `(loaded-plugins)` for `"core:stdlib"` synchronously at
*its own load time*, by design (fail fast instead of a config read silently resolving to
`#void` — see the Config helpers section below). A merely *declared*, not-yet-activated
`core:stdlib` doesn't show up in `(loaded-plugins)`, so all four need `core:stdlib` loaded
eagerly first, as above. The zero-trigger form is for consumers that only reach `core:stdlib`
via `call!` at runtime (e.g. `core:plum`'s grammar/plugin install commands) with none of
these load-time reads in the mix.

## Commands

### Selections

| Command                            | Effect                                                              |
|-------------------------------------|----------------------------------------------------------------------|
| `stdlib/single-selection?`         | `#t` if the given selection list holds exactly one selection         |
| `stdlib/all-single-char?`          | `#t` if every selection in the list spans exactly one grapheme       |
| `stdlib/cursor-char-index`         | 0-indexed head char offset of the primary selection, or `#f`         |

All three accept `#f` and return `#f` — callers only need to check `(current-selections)` for
`#f` once, at the call site, rather than re-checking inside every helper.

### Filesystem + list search

| Command                 | Effect                                                              |
|--------------------------|----------------------------------------------------------------------|
| `stdlib/find`            | First element of the given list satisfying the given predicate, or `#f` |
| `stdlib/write-file`      | Write content to a path, creating or truncating it                   |
| `stdlib/delete-dir`      | Recursively delete a directory; idempotent                           |
| `stdlib/delete-file`     | Delete a file; idempotent                                             |
| `stdlib/list-subdirs`    | Sorted basenames of a directory's subdirectories                     |

Thin wrappers over Steel's `steel/filesystem`/`steel/ports` — `core:plum` and `core:lsp`
both call into these rather than each carrying its own copy.

### Subprocess

| Command      | Effect                                                                       |
|--------------|-------------------------------------------------------------------------------|
| `stdlib/run` | Spawn a command, blocking until exit; returns `(stdout stderr exit-code)`, with `exit-code` `#f` and the failure reason in `stderr`'s place on spawn/wait failure |

`core:plum` (`plum/run!`) and `core:pickers` (`pickers/run-stdout-raw`) both build their
raise-vs-`#f` failure policy on top of this — see their own doc comments.

### Command arguments

| Command                    | Effect                                                              |
|------------------------------|----------------------------------------------------------------------|
| `stdlib/resolve-lang-arg`  | A typed language-name argument, else the current buffer's language, else `#f` after a warning naming the given command |

The typed-string-vs-integer distinction this resolves comes from how HUME's minibuffer
dispatch marshals a `:` command's argument — see the command's own doc comment.

### Plugin config

| Command                 | Effect                                                              |
|--------------------------|----------------------------------------------------------------------|
| `stdlib/config-boolean` | The given key's value in the given `#:config` hash, or the given default if absent; errors if it isn't `#t`/`#f` |
| `stdlib/config-string`  | Same, erroring if the resolved value isn't a string                  |
| `stdlib/config-enum`    | Same, erroring if the resolved value isn't in the given list of allowed symbols |

Every error names the calling plugin (its first argument) and the offending key, so a bad
`#:config` value fails at load time with a message pointing at exactly what to fix — the same
shape `core:git-diff`, `core:pickers`, and `core:vim-keybind` all use for their own config.

## How it works

HUME's scripting surface has two layers, and `core:stdlib` is the outermost:

- **prelude** — convenience macros for `init.scm`; loaded at startup
- **core:stdlib** (this plugin) — commands useful to plugin authors; loaded explicitly, like
  any other plugin, before anything that depends on it.

Cross-plugin access in HUME is `call!`-only — plugins never `require` each other's modules
(that would break the namespace isolation each plugin gets). That's why the public API here
is exposed as `define-command!`-registered commands rather than a `provide`d library.

The internal accessors (`stdlib/selection-anchor`, `stdlib/selection-head`,
`stdlib/selection-primary?`, `stdlib/primary-selection`) exist so nothing outside this file
picks apart a selection's `(anchor head primary?)` triple with raw `car`/`cadr`/`caddr` — the
triple's shape is this plugin's implementation detail, not a public contract.
