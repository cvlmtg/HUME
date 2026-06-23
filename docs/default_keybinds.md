# Default Key Bindings

All bindings listed here are the built-in defaults. Any of them can be overridden or extended in your `~/.config/hume/init.scm` using `(bind-key! ...)`.

---

## Normal Mode

### Motion

| Key | Command |
|-----|---------|
| `h` / `←` | Move left one grapheme |
| `l` / `→` | Move right one grapheme |
| `j` / `↓` | Move down one visual line |
| `k` / `↑` | Move up one visual line |

> **Ctrl+motion extend (kitty only):** `Ctrl+h/j/k/l` run the same motion with extend mode on for that keypress only (one-shot extend without toggling `e`). `Ctrl+w` and `Ctrl+b` also work the same way (extend-next-word and extend-prev-word). Requires the kitty keyboard protocol.

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
| `Ctrl+x` | Select current line (forward) — always extends (works in kitty and legacy) |
| `Ctrl+X` | Select current line (backward) — always extends (works in kitty and legacy) |

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

### Surround Add (`m w`)

Wrap each selection with a delimiter pair. Press `m w` then a character; if it's a configured pair char, the selection is wrapped with the matching open/close pair. Any other character wraps symmetrically with itself on both sides.

| Sequence | Wraps with |
|----------|------------|
| `m w (` or `)` | `( … )` |
| `m w [` or `]` | `[ … ]` |
| `m w {` or `}` | `{ … }` |
| `m w <` or `>` | `< … >` |
| `m w "` | `" … "` |
| `m w '` | `' … '` |
| `m w `` ` `` | `` ` … ` `` |
| `m w <other>` | `<other> … <other>` |

> **Note:** bare `[` and `]` (without a preceding `m` prefix) are bound to kill-ring cycling (`paste-ring-older` / `paste-ring-newer`). `m w [`, `m i [`, `m a [`, and `m s [` are unaffected.

### Edit

| Key | Command |
|-----|---------|
| `d` | Delete selections; push deleted text onto the kill ring |
| `c` | Delete selection content, push onto kill ring, enter insert mode (one undo group). A trailing newline is kept — `c` on a line rewrites its content without removing the line itself. |
| `y` | Yank selections: write to system clipboard **and** push onto kill ring |
| `p` | Smart-p paste after selection (see below) |
| `P` | Smart-p paste before selection (see below) |
| `[` | Within a paste session: cycle one step older and re-paste |
| `]` | Within a paste session: cycle one step newer and re-paste |
| `r <char>` | Replace every character in each selection with `<char>` |
| `J` | Join the current line with the next, replacing the newline with a space; cursor lands on the inserted space. If the selection spans multiple lines, all are joined into one. |
| `&` | Align all selections to the column of the primary selection's anchor. Spaces are inserted or removed at the left edge of each selection. Multi-line selections are left unchanged. |
| `u` | Undo |
| `U` / `Ctrl+r` | Redo |
| `.` | Repeat last editing action |

#### Smart-p paste heuristic

`p` and `P` decide what to paste based on the last command:

- **After `d`, `c`, or a paste/ring command** — reads the kill ring head (most recently killed or pasted text).
- **Otherwise** — reads the system clipboard.

`[` and `]` cycle within the current **paste session** (opened by a preceding `p`/`P`). Each cycle replaces the previous paste. The whole session records as one undo step. Consecutive `p` presses append copies (each is a new session and a separate undo step).

To force a specific source, use the register prefix `"`:

| Sequence | Source |
|----------|--------|
| `"cp` | System clipboard explicitly |
| `"kp` | Kill-ring head (most recent kill) |
| `"cy` | Write clipboard only (no kill ring push) |
| `"5y` / `"5p` | Write / paste in-memory register 5 (symmetric round-trip) |
| `"ky` | Push yank onto kill ring only (no clipboard) |
| `"by` | Discard (black hole) |

### Selection Manipulation

| Key | Command |
|-----|---------|
| `;` | Collapse selections to cursor, exit extend mode |
| `Ctrl+;` | Collapse selections to anchor, exit extend mode (kitty only) |
| `,` | Keep only the primary selection |
| `Ctrl+,` | Remove primary selection, promote next (kitty only) |
| `Ctrl+e` | Flip anchor and head of each selection (works in kitty and legacy) |
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
| `Q <reg>` | Start recording into named register (`0`–`9`) |
| `Q` (while recording) | Stop recording |
| `q q` | Replay register `q` |
| `q <reg>` | Replay named register (`0`–`9`) |
| `<count> q q` | Replay register `q` `<count>` times |

### Register Prefix (`"`)

Prefix any yank, delete, change, or paste command with `"<reg>` to target a specific register instead of the kill ring / clipboard.

| Prefix | Effect on the following command |
|--------|---------------------------------|
| `"c` | System clipboard (`y` writes clipboard only; `p` reads clipboard) |
| `"0` – `"9` | Symmetric in-memory storage — `"5y` writes register 5, `"5p` reads it back. No kill ring interaction. Also used as macro registers by `Q`/`q`. |
| `"k` | Kill-ring head — `"kp` pastes the most-recent kill; `"ky`/`"kd`/`"kc` push onto the ring without touching the clipboard. Older entries reachable only via `[`/`]`. |
| `"b` | Black hole — discard (only meaningful for `y`/`d`/`c`) |

The prefix is sticky across motions and text objects, but consumed (cleared) by the first register-consuming command (`y`, `d`, `c`, `p`, `P`). Press `Esc` to cancel a pending prefix.

### Jump List

| Key | Command |
|-----|---------|
| `Ctrl+o` | Jump backward in the jump list |
| `Ctrl+i` / `Tab` | Jump forward in the jump list |

### Pane Focus (`Ctrl+p` prefix)

| Sequence | Command |
|----------|---------|
| `Ctrl+p w` | Focus next pane |
| `Ctrl+p h` | Focus pane to the left |
| `Ctrl+p j` | Focus pane below |
| `Ctrl+p k` | Focus pane above |
| `Ctrl+p l` | Focus pane to the right |

### Mode Transitions

| Key | Command |
|-----|---------|
| `i` | Enter insert mode at selection start |
| `a` | Enter insert mode after selection end (stays on the line if selection ends on a newline) |
| `I` | Enter insert mode at first non-blank on line |
| `A` | Enter insert mode at end of line |
| `o` | Open new line below, enter insert mode |
| `O` | Open new line above, enter insert mode |
| `:` | Open command prompt |

---

## Extend Mode

Extend mode has a sparse keymap. Keys not listed below fall through to the normal keymap with extend active.

| Key | Command |
|-----|---------|
| `o` | Flip anchor and head of each selection |
| `Ctrl+e` | Flip anchor and head of each selection (falls through from normal mode) |

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
| `:quit` | `:q` | Close the current buffer; quit when it is the last real buffer. Add `!` to discard unsaved changes. |
| `:write` | `:w` | Write current buffer to disk |
| `:write-quit` | `:wq` | Write and quit |
| `:quit-all` | `:qa` | Quit the editor, refusing if any buffer has unsaved changes. Use `:qa!` to force. |
| `:toggle-soft-wrap` | `:wrap` | Toggle soft line wrapping |
| `:set global <key>=<value>` | | Set a global setting |
| `:set buffer <key>=<value>` | | Set a buffer-local setting override |
| `:messages` | `:mes` | Show the message log in a read-only scratch buffer |
| `:edit <path>` | `:e` | Open a file; `:e` with no arg reloads current |
| `:bnext` | `:bn` | Switch to the next open buffer |
| `:bprev` | `:bp` | Switch to the previous open buffer |
| `:buffer-delete` | `:bd` | Close the focused buffer (guards unsaved changes) |
| `:reload-config` | | Reload `init.scm` from scratch |
| `:split` | `:sp` | Split current pane horizontally *(not yet implemented)* |
| `:vsplit` | `:vsp` | Split current pane vertically *(not yet implemented)* |
| `:clear-search` | | Clear search highlights |

See [settings.md](settings.md) for the full list of available keys and values for `:set`.
