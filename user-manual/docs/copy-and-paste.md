# Copy & Paste

HUME has two paste sources: the system clipboard and a kill ring that remembers the last 10 things you deleted or yanked.

| Key | Effect |
|-----|--------|
| `y` | Yank (copy) selection: writes to the system clipboard **and** pushes onto the kill ring |
| `p` | Smart-paste after the selection |
| `P` | Smart-paste before the selection |
| `[` | Cycle one step older in the kill ring and re-paste |
| `]` | Cycle one step newer in the kill ring and re-paste |

With a real selection (more than a single character), `p` and `P` **replace** it, unless what you're pasting is already exactly the selected text, in which case they paste right alongside it instead, so pressing `p` again after a paste adds another copy rather than overwriting the first one. "After" and "before" apply on a bare cursor, or once a matching selection has collapsed this way. The replaced text (when something is replaced) is thrown away rather than pushed onto the kill ring.

## Lines and inline text

A paste remembers how its text was taken. A selection that starts at the beginning of a line and ends on a line break (what `x` selects) is taken as whole lines; any other selection is taken as inline text. Text yanked in HUME keeps this shape when you paste it back from the system clipboard, until something else changes the clipboard.

| Taken as | On a bare cursor | Over a selection |
|----------|------------------|------------------|
| Whole lines | `p` puts the lines below the cursor's line, `P` above it | The lines replace the selection. When the selection does not start its line, the pasted lines begin on a new line of their own, and the text after the selection continues on the line below them |
| Inline text | `p` puts the text after the cursor without crossing the line break, `P` before the cursor | The text replaces the selection |

Text from outside HUME, such as the system clipboard after another program wrote to it, or text a plugin supplies, has no remembered shape: it pastes as whole lines when it ends with a line break and as inline text otherwise.

`d` and `y` take what the selection covers, with one exception: the final line break of the buffer is never taken on its own. On the last line, a cursor resting on that line break (past the last character of a line with text) gives `d` and `y` nothing to take, and deleting the last line with `x` then `d` leaves no empty line behind. `c` keeps the line break a selection ends on and takes the rest as inline text.

## Smart-p paste

`p` and `P` decide what to paste based on whether the buffer has changed since the ring last did:

- **Nothing edited since the last `d`, `c`, or `y`**: reads the kill ring. Right after a `d`, `c`, or `y` that is the head (the most recently killed, changed, or yanked text); right after a paste or a `[`/`]` cycle it is the entry that paste or cycle used.
- **Something edited since**: reads the system clipboard. A first paste falls back to the kill ring head when the clipboard is empty or unavailable. Once a press has read the clipboard, a further `p` or `P` pastes nothing if the clipboard is then empty or unavailable.

Since `y` writes to both the clipboard and the kill ring, `y` then `p` pastes what you just yanked. Yanking to an explicit register instead (`"0y`) leaves both the clipboard and this "what should a bare `p` read" tracking untouched. A following bare `p` behaves exactly as it would have if the `"0y` hadn't happened. Use `"0p` to read the register back.

`[` and `]` only work inside a **paste session**, one opened by a preceding `p` or `P`. Each cycle replaces the previous paste, and the whole session records as a single undo step. Consecutive `p` presses append copies (each its own separate undo step) for the same reason pasting over a matching selection does: pasting the same text again lands next to it, not over it.

Two plain commands, `paste-after`/`paste-before`, exist alongside `p`/`P` with no key bound by default: they always read the kill-ring head, with no clipboard fallback, and always replace a real selection outright, with no same-text check. To stack a copy with plain paste, collapse the selection (`;`) before pasting. See [GUI-style paste](#gui-style-paste-bundled-plugin) below for a plugin built on them.

## Clipboard over SSH

In an SSH session, or when no clipboard server is available (a Linux machine without X11 or Wayland), HUME asks your terminal to set the clipboard instead (OSC 52). `y` then puts the text on the clipboard of the machine you are sitting at.

- Your terminal must allow programs to set the clipboard. Some ask for this in their settings, iTerm2 for one.
- Inside `tmux`, add `set -g set-clipboard on` to your `tmux.conf`.
- `p` pastes what HUME last yanked. Terminals don't let programs read the clipboard back, so text copied in another application comes in through the terminal's own paste shortcut.

## Pasting from the terminal

Pasting text from outside HUME (your system clipboard via the terminal's own paste shortcut, a mouse paste, or a paste from `tmux`/`screen`) lands in one step, however long the pasted text is.

- In Insert mode, the text is inserted at the cursor. Auto-pairing does not run on pasted text, so pasted brackets and quotes are never doubled up.
- In Normal or Extend mode, a real selection is replaced; on a bare cursor the text is inserted in front of it.
- On the command prompt, the Search and Sift prompts, and picker query fields, line breaks in the pasted text become spaces and a trailing line break is dropped, since those fields are single-line.

## Whitespace and the kill ring

When the current kill-ring head is a pure-whitespace entry (nothing but whitespace characters of any kind, such as spaces, tabs, and line breaks), the next delete, change, or yank overwrites that slot in place instead of taking a fresh one. This stops the ring filling up with entries you'd never want to cycle back to. To keep whitespace durably, yank it into a numbered register (`"0`–`"9`).

The ring never holds two identical entries: entries with the same text and the same shape (whole lines or inline). Deleting, changing, or yanking text that's already in the ring moves that entry back to the front instead of adding a duplicate, so cycling with `[`/`]` never repeats the same entry twice.

## Register prefix (`"`)

Prefix a yank, delete, change, or paste with `"` + a register name to target a specific source or destination:

| Example | Effect |
|---------|--------|
| `"cp` | Paste from the system clipboard explicitly |
| `"kp` | Paste from the kill-ring head |
| `"ky` | Yank to the kill ring only, leaving the clipboard alone |
| `"0y` | Yank to register 0 |
| `"5p` | Paste from register 5 |
| `"bd` | Delete to the black hole (nothing saved) |

Four kinds of register are addressable via `"`:

| Register | Contents |
|----------|----------|
| `0`–`9` | Numbered storage: `"5y` writes and `"5p` reads the same slot |
| `k` | Kill-ring head |
| `c` | System clipboard |
| `b` | Black hole: writes are discarded, reads return nothing |

::: warning Numbered registers are shared with macros
`"3y` and `Q 3` write to the same slot, and the last write wins: recording a macro into `3` overwrites text you stored there, and yanking into `3` destroys the macro. Keep the two uses on separate numbers.
:::

Two further registers exist but cannot be named through the `"` prefix:

| Register | How it's used |
|----------|---------------|
| `q` | Default macro register: written by `Q` recording, read by `q` replay |
| `s` | Search register: holds the last search pattern; written by `/`, `?`, `*`, and reused when you repeat the search |

## GUI-style paste (bundled plugin)

If you'd rather keep the clipboard and the kill ring on separate keys instead of letting `p` choose, load `core:classic-paste`. It rebinds `p` / `P` to always paste from the kill ring, and binds `Ctrl-v` / `Ctrl-Shift-v` to always paste from the system clipboard (the latter needs the kitty protocol).

```scheme
(load-plugin! "core:classic-paste")
```
