# core:vim-keybind

Vim muscle-memory keybindings: line-motion keys, the `C`/`D` composites HUME doesn't bind
natively, and the visual-mode `o` flip alias.

## Usage

```scheme
(declare-plugin! "core:stdlib")
(load-plugin! "core:vim-keybind" #:config (hash "change-to-eol" 'smart))
```

- **Depends on:** `core:stdlib`: config validation (`"change-to-eol"`) calls
  `stdlib/config-enum` via `call!` at this plugin's own load time.
- **Activates on:** its own key bindings only. Most of what it rebinds
  (`goto-line-start`, `goto-line-end`, …) are built-in commands with no typed form or
  hook of their own, so it has no `manifest.scm` and must be loaded eagerly (see the
  [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-vim-keybind)
  for what each `"change-to-eol"` value does.

## Commands

| Command | Effect |
|---|---|
| `vim-change-to-eol` | Change from the cursor to the end of the line |
| `vim-change-to-eol-or-copy-line` | `'smart`-mode dispatch bound to `C`: change to end of line on a bare cursor with no count, else copy the selection onto the line(s) below |
| `vim-delete-to-eol` | Delete from the cursor to the end of the line |

## How it works

### `C` — change to end of line

| `"change-to-eol"` | `C` binds to | Behavior |
|---|---|---|
| `'on` | `vim-change-to-eol` | Always changes to end of line, ignoring the selection |
| `'smart` (default) | `vim-change-to-eol-or-copy-line` | Context-sensitive (see below) |
| `'off` | *(unbound)* | HUME's native `copy-selection-on-next-line` stays reachable on it |

`vim-change-to-eol-or-copy-line` takes the injected `count` (`0` means no count was
typed) and, only when it's `0`, calls `stdlib/all-single-char?` to tell a bare cursor
from a real selection. A bare cursor with no count delegates to `vim-change-to-eol`
(`goto-line-end` extend, then `change`); any count prefix, or a real selection with no
count, calls `copy-selection-on-next-line` directly with the count forwarded.

> [!NOTE]
> `stdlib/all-single-char?`'s "bare cursor" reads as `anchor == head`. The editor's
> `select-inserted-text` setting (on by default) makes this false right after typing
> something in Insert mode: leaving Insert selects the run you just typed instead of
> leaving a plain cursor. So `i foo <Esc> C` in `'smart` mode copies the selection onto
> the line below rather than changing to end-of-line: `C` still reads a bare cursor
> correctly, it's just that `Esc` no longer always leaves one. `'on` sidesteps this
> entirely by ignoring what the selection looks like.

Dot-repeat needs no `#:repeatable` annotation on either wrapper command: `change` and
`delete` are natively repeatable and capture the preceding `goto-line-end` (extend) step
themselves, via the shared selection-recipe accumulator, regardless of whether the
wrapper that invoked them is flagged repeatable.

### `o` — flip selection

Restores vim's visual-mode "flip the selection" gesture, bound in Extend mode. HUME's
native `Ctrl-e` already flips in any mode (including Normal) and works on legacy
terminals, so `o` is purely a muscle-memory alias, not new capability.

### `Ctrl-6` — alternate buffer

The portable form of vim's `Ctrl-^`; both share a keycap on US layouts and emit
identical bytes. Under the kitty keyboard protocol this arrives as `Char('6')` +
`CONTROL`; legacy terminals emit `0x1E`, which HUME does not currently surface as this
binding (falls back to `:e #` on those terminals).

### Why `G` isn't bound

Vim's `G` (last line) is exactly the kind of key this plugin exists to restore, but it's
left alone: `G` is a prefix in HUME's own keymap (`G L`/`G U`/`G C` case
transforms, plus `G R` rename from `core:lsp`), and binding a single bare key replaces
the whole trie node it lands on. Rebinding `G` here would silently take those three (and
`G R`) down with it for anyone who loads this plugin. `g e` reaches the last line and is
unaffected, so the trade is one alias against three-to-four working sequences.

## Design decisions

- **Check `(declared-plugins)` for `core:stdlib` at load time, unconditionally, even
  though only `'smart` mode calls into it at runtime.** Config resolution itself
  (`stdlib/config-enum`) calls into `core:stdlib`, so every mode needs the dependency
  present at load. Checking at load time turns a missing dependency into a load error
  naming `core:stdlib`, rather than a wrong-branch bug that only surfaces at the first
  `C` keypress.
