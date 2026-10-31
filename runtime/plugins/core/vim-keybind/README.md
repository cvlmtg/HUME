# core:vim-keybind

Vim muscle-memory keybindings: line-motion keys, the `C`/`D` composites HUME does not bind
natively, `o` to flip the selection in Extend mode, and `Ctrl-6` for the alternate buffer.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:vim-keybind" #:config (hash 'change-to-eol 'smart))
```

- **Depends on:** `core:stdlib`, at load time to validate `'change-to-eol`, so a missing
  `core:stdlib` is a load error.
- **Activates on:** `load-plugin!`. It mostly binds built-in commands, which a manifest
  cannot claim, so it has no `manifest.scm` (see the
  [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-vim-keybind).

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'change-to-eol` | `'on` \| `'smart` \| `'off` | `'smart` | What `C` binds to: `'on`, `vim-keybind/change-to-eol`; `'smart`, `vim-keybind/change-to-eol-or-copy-line`; `'off`, nothing, so HUME's own `C` stays |

## Key bindings

| Mode | Key | Command | Effect |
|---|---|---|---|
| Normal | `0` | `goto-line-start` | Start of line |
| Normal | `^` | `goto-first-nonblank` | First non-blank character |
| Normal | `$` | `goto-line-end` | End of line |
| Normal | `C` | per `'change-to-eol` | See [`C` and `D`](#c-and-d) |
| Normal | `D` | `vim-keybind/delete-to-eol` | Delete from the cursor to the end of the line |
| Normal | `Ctrl-6` | `goto-alternate-buffer` | Alternate buffer |
| Extend | `o` | `flip-selections` | Swap the selection's ends |

## How it works

### `C` and `D`

`vim-keybind/delete-to-eol` and `vim-keybind/change-to-eol` extend to the end of the line
with `goto-line-end`, then call `delete` or `change`.

`vim-keybind/change-to-eol-or-copy-line` takes the injected `count` (`0` means no count was
typed) and, only when it is `0`, calls `stdlib/all-single-char?` to tell a bare cursor from
a real selection. A bare cursor with no count delegates to `vim-keybind/change-to-eol`. A
count prefix, or a real selection with no count, calls `copy-selection-on-next-line` with
the count forwarded, so HUME's own `C` stays reachable.

### Dot-repeat

Neither `C` nor `D` needs a `#:repeatable` annotation. `change` and `delete` are natively
repeatable and capture the preceding `goto-line-end` (extend) step through the shared
selection-recipe accumulator, whether or not the wrapper that invoked them is flagged
repeatable.

### `o`

Restores vim's visual-mode "flip the selection" gesture in Extend mode. It is an alias for
HUME's native `Ctrl-e`, which the Normal keymap binds and Extend mode reaches by falling
through to it.

### `Ctrl-6`

The portable form of vim's `Ctrl-^`; both share a keycap on US layouts. Under the kitty
keyboard protocol it arrives as `Char('6')` with `CONTROL`. A legacy terminal sends `0x1E`,
which termina's decoder also reports as `Char('6')` with `CONTROL`, so the same binding
matches.

## Design decisions

- **`G` left unbound, not mapped to vim's last line**, because `G` is a prefix in HUME's
  own keymap that groups edit commands, and binding a bare key over a prefix hides every
  binding under it. `g e` reaches the last line.

## Known limitations

- **`'smart` reads a just-typed run as a selection.** `stdlib/all-single-char?` tests for
  `anchor == head`, and with `insert-select-on-exit` (on by default) leaving Insert selects
  the run just typed. So `i foo <Esc> C` copies that selection onto the line below instead
  of changing to end of line. `'on` does not look at the selection.
