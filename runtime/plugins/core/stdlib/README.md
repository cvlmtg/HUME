# core:stdlib

General-purpose standard library for plugin authors — a growing toolkit of commands any
plugin might need, exposed via `call!` so cross-plugin code never has to re-derive them.

## Usage

```scheme
(load-plugin "core:stdlib")
```

Load before any plugin that calls its commands (e.g. `core:vim-keybind`).

`(declare-plugin "core:stdlib")` also works: with no explicit `#:commands`/`#:events`/
`#:languages`, it reads `core:stdlib`'s own `manifest.scm`, which declares all three commands
below as activation triggers — `call!` dispatch activates an unactivated plugin inline before
retrying the call, so a lazily-declared `core:stdlib` still activates transparently the first
time another plugin's command body calls one of them at runtime.

**Caveat**: `core:vim-keybind`'s `'smart` `change-to-eol` mode checks `(loaded-plugins)` for
`"core:stdlib"` synchronously at *`core:vim-keybind`'s own load time*, by design (fail fast
instead of a wrong-branch bug at the first `C` keypress — see that plugin's README). A merely
*declared*, not-yet-activated `core:stdlib` doesn't show up in `(loaded-plugins)`, so `'smart`
mode still needs `core:stdlib` loaded eagerly first, as above. The zero-trigger form is for
consumers that only reach `core:stdlib` via `call!` at runtime (e.g. `core:lsp`'s diagnostics
navigation) with no `core:vim-keybind` `'smart` mode in the mix.

## Commands

### Selections

| Command                            | Effect                                                              |
|-------------------------------------|----------------------------------------------------------------------|
| `stdlib/single-selection?`         | `#t` if the given selection list holds exactly one selection         |
| `stdlib/all-single-char?`          | `#t` if every selection in the list spans exactly one grapheme       |
| `stdlib/cursor-char-index`         | 0-indexed head char offset of the primary selection, or `#f`         |

All three accept `#f` and return `#f` — callers only need to check `(current-selections)` for
`#f` once, at the call site, rather than re-checking inside every helper.

## How it works

HUME's scripting surface has two layers, and `core:stdlib` is the outermost:

- **prelude** — convenience macros for `init.scm`; loaded at startup
- **core:stdlib** (this plugin) — commands useful to plugin authors; loaded explicitly, like
  any other plugin, before anything that depends on it.

Cross-plugin access in HUME is `call!`-only — plugins never `require` each other's modules
(that would break the namespace isolation each plugin gets). That's why the public API here
is exposed as three `define-command!`-registered commands rather than a `provide`d library.

The internal accessors (`stdlib/selection-anchor`, `stdlib/selection-head`,
`stdlib/selection-primary?`, `stdlib/primary-selection`) exist so nothing outside this file
picks apart a selection's `(anchor head primary?)` triple with raw `car`/`cadr`/`caddr` — the
triple's shape is this plugin's implementation detail, not a public contract.
