# Default Key Bindings

All bindings listed here are the built-in defaults. Any of them can be overridden or extended in your `~/.config/hume/init.scm` using `(keymap-bind! ...)`.

---

## Normal Mode

### Motion

| Key | Command |
|-----|---------|
| `h` / `←` | Move left one grapheme |
| `l` / `→` | Move right one grapheme |
| `j` / `↓` | Move down one visual line |
| `k` / `↑` | Move up one visual line |

> **Ctrl+motion extend (kitty only):** When the kitty keyboard protocol is active, `Ctrl+h/j/k/l` run the same motion with extend mode on for that keypress only (one-shot extend without toggling `e`). This is handled at runtime and not visible in the keymap.

### Word Motion

| Key | Command |
|-----|---------|
| `w` | Select next word |
| `W` | Select next WORD (whitespace-delimited) |
| `b` | Select previous word |
| `B` | Select previous WORD |

### Line Start / End

| Key | Command |
|-----|---------|
| `0` / `Home` | Go to line start |
| `$` / `End` | Go to line end (last character) |
| `^` | Go to first non-blank character on line |

### Paragraph

| Key | Command |
|-----|---------|
| `{` | Move to previous paragraph start |
| `}` | Move to next paragraph start |

### Line Selection

| Key | Command |
|-----|---------|
| `x` | Select current line (forward) |
| `X` | Select current line (backward) |
| `Ctrl+x` | Select current line (forward) — extend in extend mode |
| `Ctrl+X` | Select current line (backward) — extend in extend mode |

### Page Scroll

| Key | Command |
|-----|---------|
| `PageDown` | Scroll down one viewport |
| `PageUp` | Scroll up one viewport |
| `Ctrl+d` | Scroll down half a viewport |
| `Ctrl+u` | Scroll up half a viewport |

### Goto (`g` prefix)

| Sequence | Command |
|----------|---------|
| `g g` | Go to first line of buffer |
| `g e` | Go to last line of buffer |
| `g h` | Go to line start |
| `g l` | Go to line end |
| `g s` | Go to first non-blank character on line |

### Find / Till Character (wait-char)

The next keypress after `f`/`F`/`t`/`T` is consumed as the target character.

| Key | Command |
|-----|---------|
| `f <char>` | Find `<char>` forward (inclusive) |
| `F <char>` | Find `<char>` backward (inclusive) |
| `t <char>` | Till `<char>` forward (stops before it) |
| `T <char>` | Till `<char>` backward (stops after it) |
| `=` | Repeat last find/till forward |
| `-` | Repeat last find/till backward |

### Search

| Key | Command |
|-----|---------|
| `/` | Open forward search prompt |
| `?` | Open backward search prompt |
| `n` | Next search match (absolute direction) |
| `N` | Previous search match (absolute direction) |
| `s` | Select regex matches within current selections |
| `*` | Use primary selection text as search pattern |
| `m /` | Turn all search matches in the buffer into selections |

### Text Objects (`m i` / `m a`)

Text object commands collapse or extend the selection to cover an object. `m i` selects the inner content; `m a` selects including the delimiters or surrounding whitespace.

| Sequence | Inner | Around |
|----------|-------|--------|
| `m i w` / `m a w` | Inner word | Word + surrounding whitespace |
| `m i W` / `m a W` | Inner WORD | WORD + surrounding whitespace |
| `m i (` or `)` / `m a (` or `)` | Inside `()` | Including `()` |
| `m i [` or `]` / `m a [` or `]` | Inside `[]` | Including `[]` |
| `m i {` or `}` / `m a {` or `}` | Inside `{}` | Including `{}` |
| `m i <` or `>` / `m a <` or `>` | Inside `<>` | Including `<>` |
| `m i "` / `m a "` | Inside `"..."` | Including `"..."` |
| `m i '` / `m a '` | Inside `'...'` | Including `'...'` |
| `m i `` ` `` / `m a `` ` `` | Inside `` `...` `` | Including `` `...` `` |
| `m i a` / `m a a` | Argument (trimmed) | Argument + separator comma |
| `m i l` / `m a l` | Line content (no newline) | Full line (including newline) |

### Surround (`m s`)

Selects both delimiters of the surrounding pair (useful before `d`, `r`, etc.).

| Sequence | Selects |
|----------|---------|
| `m s (` or `)` | Surrounding `()` |
| `m s [` or `]` | Surrounding `[]` |
| `m s {` or `}` | Surrounding `{}` |
| `m s <` or `>` | Surrounding `<>` |
| `m s "` | Surrounding `"..."` |
| `m s '` | Surrounding `'...'` |
| `m s `` ` `` | Surrounding `` `...` `` |

### Edit

| Key | Command |
|-----|---------|
| `d` | Yank into default register, then delete selections |
| `c` | Yank, delete selections, enter insert mode (one undo group) |
| `y` | Yank selections into default register |
| `p` | Paste after selection |
| `P` | Paste before selection |
| `r <char>` | Replace every character in each selection with `<char>` |
| `u` | Undo |
| `U` / `Ctrl+r` | Redo |
| `.` | Repeat last editing action |

### Selection Manipulation

| Key | Command |
|-----|---------|
| `;` | Collapse selections to cursor, exit extend mode |
| `,` | Keep only the primary selection |
| `Ctrl+,` | Remove primary selection, promote next (kitty only) |
| `S` | Split multi-line selections — one selection per line |
| `(` | Cycle primary selection backward |
| `)` | Cycle primary selection forward |
| `C` | Duplicate each selection on the line below |
| `_` | Trim leading/trailing whitespace from each selection |
| `%` | Select entire buffer |

### Extend Mode

| Key | Command |
|-----|---------|
| `e` | Toggle sticky extend mode on/off |

In extend mode, all motion and selection commands grow the selection rather than replacing it. The mode indicator in the status line changes to `EXTEND`.

### Macros

Macros are stored in registers. Register `q` is the default.

| Key | Action |
|-----|--------|
| `Q Q` | Start recording into register `q` |
| `Q <reg>` | Start recording into named register (any alphanumeric char) |
| `Q` (while recording) | Stop recording |
| `q q` | Replay register `q` |
| `q <reg>` | Replay named register |
| `<count> q q` | Replay register `q` `<count>` times |

### Jump List

| Key | Command |
|-----|---------|
| `Ctrl+o` | Jump backward in the jump list |
| `Ctrl+i` / `Tab` | Jump forward in the jump list |

### Mode Transitions

| Key | Command |
|-----|---------|
| `i` | Enter insert mode at selection start |
| `a` | Enter insert mode after selection end |
| `I` | Enter insert mode at first non-blank on line |
| `A` | Enter insert mode at end of line |
| `o` | Open new line below, enter insert mode |
| `O` | Open new line above, enter insert mode |
| `:` | Open command prompt |
| `Ctrl+c` | Quit (force, no unsaved-changes check) |

---

## Extend Mode

Extend mode has a sparse keymap. Keys not listed below fall through to the normal keymap with extend active.

| Key | Command |
|-----|---------|
| `o` | Flip anchor and head of each selection |

All other keys behave as in normal mode, but motions and selections grow the current selection instead of replacing it.

---

## Insert Mode

| Key | Action |
|-----|--------|
| `Esc` / `Ctrl+c` | Return to normal mode |
| `←` / `→` / `↑` / `↓` | Move cursor |
| `Home` | Go to line start |
| `End` | Go to line end |
| `Backspace` | Delete character before cursor (auto-pairs aware) |
| `Delete` | Delete character under cursor |
| `Enter` | Insert newline (auto-pairs aware) |
| Any other character | Insert character (auto-pairs aware) |

Auto-pairs: when `auto-pairs-enabled` is on, typing an opening delimiter (`(`, `[`, `{`, `"`, `'`, `` ` ``) inside an empty selection automatically inserts the closing character. Typing a closing delimiter when the cursor is directly before it skips over it instead of inserting a duplicate.

---

## Typed Commands (`:` prompt)

| Command | Aliases | Description |
|---------|---------|-------------|
| `:quit` | `:q` | Quit the editor |
| `:write` | `:w` | Write current buffer to disk |
| `:write-quit` | `:wq` | Write and quit |
| `:toggle-soft-wrap` | `:wrap` | Toggle soft line wrapping |
| `:set global <key>=<value>` | | Set a global setting |
| `:set buffer <key>=<value>` | | Set a buffer-local setting override |
| `:clear-search` | `:cs` | Clear search highlights |

See [settings.md](settings.md) for the full list of available keys and values for `:set`.
