# core:vim-keybind

Vim muscle-memory keybindings — line-motion keys, C/D/G composites HUME doesn't bind
natively, and the visual-mode `o` flip alias.

## Usage

```scheme
(declare-plugin "core:stdlib")
(load-plugin "core:vim-keybind" #:config (hash "change-to-eol" 'smart))
```

Loads eagerly: most of what it rebinds (`goto-line-start`, `goto-last-line`, …) are built-in
commands, not plugin commands, so there's no dispatch that could ever trigger it if it were
declared lazily. Requires `core:stdlib` declared or loaded first — config validation
(`"change-to-eol"`) calls `stdlib/config-enum` via `call!` at this plugin's own load time,
and a bare `(declare-plugin "core:stdlib")` is enough since `call!`'s lazy-miss retry
inline-activates it before the config read runs. See
[Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-vim-keybind) for what
each `"change-to-eol"` value does.

## Commands

| Command | Effect |
|---|---|
| `vim-change-to-eol` | Change from the cursor to the end of the line |
| `vim-change-to-eol-or-copy-line` | `'smart`-mode dispatch bound to `C`: change to end of line on a bare cursor with no count, else copy the selection onto the line(s) below |
| `vim-delete-to-eol` | Delete from the cursor to the end of the line |

## How it works

`C`'s binding depends on `"change-to-eol"` (`core:stdlib`'s `stdlib/config-enum`, defaulting
to `'smart`): `'on` binds `vim-change-to-eol` unconditionally; `'smart` binds
`vim-change-to-eol-or-copy-line`, the context-sensitive dispatch below; `'off` leaves `C`
unbound, so HUME's native `copy-selection-on-next-line` stays reachable on it. Because config
resolution itself calls into `core:stdlib`, the plugin checks `(declared-plugins)` for
`"core:stdlib"` unconditionally at load time, before reading config at all — every mode needs
`core:stdlib` now, not just `'smart` (which additionally calls `stdlib/all-single-char?` at
runtime, below). Checking at load time means a missing dependency is a load error naming
`core:stdlib`, not a wrong-branch bug that only surfaces at the first `C` keypress.

`vim-change-to-eol-or-copy-line` takes the injected `count` (`0` when no count was typed)
and, only when it's `0`, calls `stdlib/all-single-char?` via `call!` to tell a bare cursor
from a real selection. On a bare cursor with no count it delegates to `vim-change-to-eol`
(`goto-line-end` extend, then `change`); any count prefix, or a real selection with no count,
calls `copy-selection-on-next-line` directly with the count forwarded.

Dot-repeat needs no `#:repeatable` annotation on the wrapper commands: `change` and `delete`
are natively repeatable and capture the preceding `goto-line-end` (extend) step themselves,
via the shared selection-recipe accumulator, regardless of whether the wrapper that invoked
them is flagged repeatable.

`o` (bound in Extend mode) restores vim's visual-mode "flip the selection" gesture. HUME's
native `Ctrl+e` already flips in any mode — including Normal — and works on legacy terminals,
so `o` is purely a muscle-memory alias, not new capability.

`Ctrl+6` is the portable form of vim's `Ctrl+^` — both share a keycap on US layouts and emit
identical bytes. Under the kitty keyboard protocol this arrives as `Char('6')` + `CONTROL`;
legacy terminals emit `0x1E`, which HUME does not currently surface as this binding (falls
back to `:e #` on those terminals).
