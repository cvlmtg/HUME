# core:vim-keybind

Vim muscle-memory keybindings — line-motion keys plus C/D/G composites HUME doesn't bind
natively.

## Usage

Requires `core:stdlib` loaded first — the default `'smart` `change-to-eol` mode hard-errors at
load time otherwise (see [Config](#config) to opt out with `'on`/`'off` instead):


```scheme
(load-plugin "core:stdlib")
(load-plugin "core:vim-keybind")
```

## Keys

| Key      | Command                            | Native equivalent |
|----------|--------------------------------------|--------------------|
| `0`      | goto-line-start                     | `g h`              |
| `^`      | goto-first-nonblank                 | `g s`              |
| `$`      | goto-line-end                       | `g l`              |
| `Ctrl+6` | goto-alternate-file                 | `:b#`              |
| `C`      | vim-change-to-eol-or-copy-line*     | `ctrl-g l c`       |
| `D`      | vim-delete-to-eol                   | `ctrl-g l d`       |
| `G`      | goto-last-line                      | `g e`              |

\* `C` is context-sensitive: on a bare cursor it's vim's change-to-end-of-line; with a real
(multi-char) selection already active, it instead falls back to HUME's native
`copy-selection-on-next-line`, so that command stays reachable without giving up vim muscle
memory for the common (bare-cursor) case.

## Config

| Key                | Value     | Effect                                                                 |
|---------------------|-----------|--------------------------------------------------------------------------|
| `"change-to-eol"`  | `'on`     | `C` always changes to end of line, ignoring any active selection.         |
| `"change-to-eol"`  | `'smart` (default) | `C` is context-sensitive: bare cursor changes to EOL, real selection copies to next line. |
| `"change-to-eol"`  | `'off`    | `C` is left unbound; HUME's default `copy-selection-on-next-line` stays reachable on `C`. |

```scheme
(load-plugin "core:stdlib")
(load-plugin "core:vim-keybind" #:config (hash "change-to-eol" 'off))
```

Use `'off` if vim muscle memory for `C` conflicts with your own use of
`copy-selection-on-next-line`. Use `'on` if you want pure vim `C` semantics
and never rely on the multicursor copy behavior.

## How it works

`vim-change-to-eol-or-copy-line` reads `(current-selections)` and calls
`stdlib/all-single-char?` via `call!` to tell a bare cursor from a real selection — this is
why `core:stdlib` must load first. On a bare cursor it delegates to
`vim-change-to-eol` (`goto-line-end` extend, then `change`); otherwise it calls
`copy-selection-on-next-line` directly.

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
