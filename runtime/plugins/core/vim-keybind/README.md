# core:vim-keybind

Vim muscle-memory keybindings — line-motion keys, C/D/G composites HUME doesn't bind
natively, and the visual-mode `o` flip alias.

## Usage

Requires `core:stdlib` loaded **eagerly** first — not just declared. The default `'smart`
`change-to-eol` mode checks `(loaded-plugins)` for `"core:stdlib"` at *this plugin's own load
time*, and a merely lazily-declared `core:stdlib` doesn't show up there until something
activates it. Loading it eagerly guarantees it's already up before this check runs (see
[Config](#config) to opt out with `'on`/`'off` instead, which don't need `core:stdlib` at all):

```scheme
(load-plugin "core:stdlib")
(load-plugin "core:vim-keybind")
```

## Keys

| Key               | Command                            | Native equivalent  |
|-------------------|------------------------------------|--------------------|
| `0`               | goto-line-start                    | `g h`              |
| `^`               | goto-first-nonblank                | `g s`              |
| `$`               | goto-line-end                      | `g l`              |
| `Ctrl+6`          | goto-alternate-file                | `:b#`              |
| `C`               | vim-change-to-eol-or-copy-line*    | `ctrl-g l c`       |
| `D`               | vim-delete-to-eol                  | `ctrl-g l d`       |
| `G`               | goto-last-line                     | `g e`              |
| `o` (Extend mode) | flip-selections                    | `Ctrl+e`           |
|-------------------|------------------------------------|--------------------|

\* `C` is context-sensitive: a bare `C` on a collapsed cursor is vim's change-to-end-of-line;
a real (multi-char) selection already active, or *any* count prefix (`1C`, `3C`, …), instead
falls back to HUME's native `copy-selection-on-next-line` (with the count forwarded, so `3C`
duplicates onto the 3 lines below), so that command stays fully reachable without giving up
vim muscle memory for the common (bare-cursor, no-count) case.

`o` restores vim's visual-mode "flip the selection" gesture in Extend mode. HUME's native
`Ctrl+e` already flips in any mode — including Normal — and works on legacy terminals, so `o`
is purely a muscle-memory alias, not new capability.

## Config

| Key                | Value     | Effect                                                                 |
|---------------------|-----------|--------------------------------------------------------------------------|
| `"change-to-eol"`  | `'on`     | `C` always changes to end of line, ignoring any active selection.         |
| `"change-to-eol"`  | `'smart` (default) | `C` is context-sensitive: bare cursor with no count changes to EOL; a real selection or any count prefix copies to the next (or `n`) line(s). |
| `"change-to-eol"`  | `'off`    | `C` is left unbound; HUME's default `copy-selection-on-next-line` stays reachable on `C`. |

```scheme
(load-plugin "core:stdlib")
(load-plugin "core:vim-keybind" #:config (hash "change-to-eol" 'off))
```

Use `'off` if vim muscle memory for `C` conflicts with your own use of
`copy-selection-on-next-line`. Use `'on` if you want pure vim `C` semantics
and never rely on the multicursor copy behavior.

## How it works

`vim-change-to-eol-or-copy-line` takes the injected `count` (`0` when no count was typed) and,
only when it's `0`, calls `stdlib/all-single-char?` via `call!` to tell a bare cursor from a
real selection — this is why `core:stdlib` must load first. On a bare cursor with no count it
delegates to `vim-change-to-eol` (`goto-line-end` extend, then `change`); any count prefix, or
a real selection with no count, calls `copy-selection-on-next-line` directly with the count
forwarded.

Dot-repeat needs no `#:repeatable` annotation on the wrapper commands: `change` and `delete`
are natively repeatable and capture the preceding `goto-line-end` (extend) step themselves,
via the shared selection-recipe accumulator, regardless of whether the wrapper that invoked
them is flagged repeatable.

`Ctrl+6` is the portable form of vim's `Ctrl+^` — both share a keycap on US layouts and emit
identical bytes. Under the kitty keyboard protocol this arrives as `Char('6')` + `CONTROL`;
legacy terminals emit `0x1E`, which HUME does not currently surface as this binding (fall
back to `:e #` on those terminals).

The `#:config` read follows the standard pattern: `(plugin-config)` returns the hash passed
at `load-plugin` time (or an empty one). `"change-to-eol"` is checked with `hash-contains?`
before `hash-ref` so a config-less load defaults to `'smart` instead of erroring.

Only `'smart` needs `core:stdlib` (`vim-change-to-eol-or-copy-line` is the only command that
calls it); the plugin checks `(loaded-plugins)` for `"core:stdlib"` at load time and errors
immediately if it's resolved to `'smart` without it — otherwise a missing dependency would
only surface once at the first `C` keypress, as a wrong-branch bug instead of a load error.
