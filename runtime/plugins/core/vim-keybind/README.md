# core:vim-keybind

Vim muscle-memory keybindings: line-motion keys, the `C`/`D` composites HUME does not bind
natively, `o` to flip the selection in Extend mode, and `Ctrl-6` for the alternate buffer.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:vim-keybind" #:config (hash "change-to-eol" 'smart))
```

- **Depends on:** `core:stdlib`: config validation (`"change-to-eol"`) calls
  `stdlib/config-enum` via `call!` at this plugin's own load time. The dependency is checked
  with `(declared-plugins)` at load for every `"change-to-eol"` value, so a missing
  `core:stdlib` is a load error that names it.
- **Activates on:** its own key bindings only. Most of what it rebinds
  (`goto-line-start`, `goto-line-end`, …) are built-in commands with no typed form or
  hook of their own, so it has no `manifest.scm` and must be loaded eagerly (see the
  [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-vim-keybind)
  for what each `"change-to-eol"` value does.

## Key bindings

| Mode | Key | Command | Effect |
|---|---|---|---|
| Normal | `0` | `goto-line-start` | Start of line |
| Normal | `^` | `goto-first-nonblank` | First non-blank character |
| Normal | `$` | `goto-line-end` | End of line |
| Normal | `C` | per `"change-to-eol"` | See below |
| Normal | `D` | `vim-delete-to-eol` | Delete from the cursor to the end of the line |
| Normal | `Ctrl-6` | `goto-alternate-buffer` | Alternate buffer |
| Extend | `o` | `flip-selections` | Swap the selection's ends |

`vim-change-to-eol` changes from the cursor to the end of the line. `vim-delete-to-eol`
and `vim-change-to-eol` both extend to the end of the line with `goto-line-end`, then call
`delete` or `change`.

### `C`

| `"change-to-eol"` | `C` binds to | Behavior |
|---|---|---|
| `'on` | `vim-change-to-eol` | Always changes to end of line, ignoring the selection |
| `'smart` (default) | `vim-change-to-eol-or-copy-line` | Context-sensitive (see below) |
| `'off` | none | HUME's own binding for `C` stays in place |

`vim-change-to-eol-or-copy-line` takes the injected `count` (`0` means no count was
typed) and, only when it is `0`, calls `stdlib/all-single-char?` to tell a bare cursor from
a real selection. A bare cursor with no count delegates to `vim-change-to-eol`. A count
prefix, or a real selection with no count, calls `copy-selection-on-next-line` with the
count forwarded.

> [!NOTE]
> `stdlib/all-single-char?` reads a bare cursor as `anchor == head`. The editor's
> `select-inserted-text` setting (on by default) makes that false right after typing in
> Insert mode: leaving Insert selects the run just typed instead of leaving a plain
> cursor. So `i foo <Esc> C` in `'smart` mode copies the selection onto the line below
> instead of changing to end of line. `'on` does not look at the selection.

## How it works

### Dot-repeat

Neither `C` nor `D` needs a `#:repeatable` annotation. `change` and `delete` are natively
repeatable and capture the preceding `goto-line-end` (extend) step through the shared
selection-recipe accumulator, whether or not the wrapper that invoked them is flagged
repeatable.

### `o`

Restores vim's visual-mode "flip the selection" gesture in Extend mode. HUME already has a flip
binding that works in every mode and on legacy terminals, so `o` is a muscle-memory alias
and adds no capability.

### `Ctrl-6`

The portable form of vim's `Ctrl-^`; both share a keycap on US layouts and emit the same
bytes. Under the kitty keyboard protocol it arrives as `Char('6')` with `CONTROL`. Legacy
terminals emit `0x1E`, which HUME does not surface as this binding.

## Design decisions

`G` is not bound. Vim's `G` (last line) is the kind of key this plugin restores, but `G` is
a prefix in HUME's own keymap that groups edit commands, and binding a bare key replaces
the whole trie node it lands on. `g e` reaches the last line.
