# Copy & Paste

HUME has two paste sources: the system clipboard and a kill ring that remembers the last 10 things you deleted or yanked.

| Key | Effect |
|-----|--------|
| `y` | Yank (copy) selection: writes to the system clipboard **and** pushes onto the kill ring |
| `p` | Smart-paste after the selection |
| `P` | Smart-paste before the selection |
| `[` | Cycle one step older in the kill ring and re-paste |
| `]` | Cycle one step newer in the kill ring and re-paste |

On a bare cursor, `p` pastes after it and `P` before it. A real selection (more than a single character) is **replaced**, and the replaced text is thrown away rather than pushed onto the kill ring.

::: details Pasting the text that is already selected
When the text you're pasting matches the selected text, `p` and `P` paste after or before the selection instead of replacing it. Pressing `p` again after a paste therefore adds another copy rather than overwriting the first.
:::

## Lines and inline text

A paste remembers how its text was taken. A selection that starts at the beginning of a line and ends on a line break (what `x` selects) is taken as whole lines; any other selection as inline text. Text yanked in HUME keeps this shape when you paste it back from the system clipboard, until something else changes the clipboard.

| Taken as | On a bare cursor | Over a selection |
|----------|------------------|------------------|
| Whole lines | `p` puts the lines below the cursor's line, `P` above it | The lines replace the selection. When the selection does not start its line, the pasted lines begin on a new line of their own, and the text after the selection continues on the line below them |
| Inline text | `p` puts the text after the cursor without crossing the line break, `P` before the cursor | The text replaces the selection |

Text from outside HUME, such as the system clipboard after another program wrote to it, or text a plugin supplies, has no remembered shape. It pastes as whole lines when it ends with a line break, and as inline text otherwise.

`d` and `y` take what the selection covers, with one exception: they never take the buffer's final line break on its own. On the last line, a cursor resting on that line break (past the last character of a line with text) gives `d` and `y` nothing to take, and deleting the last line with `x` then `d` leaves no empty line behind. `c` keeps the line break a selection ends on and takes the rest as inline text.

## Smart-p paste

`p` and `P` choose what to paste by whether the buffer has changed since the ring last did:

- **Nothing edited since the last `d`, `c`, or `y`**: reads the kill ring. Right after a `d`, `c`, or `y` that is the head (the most recently killed, changed, or yanked text). Right after a paste or a `[`/`]` cycle, it is the entry that paste or cycle used.
- **Something edited since**: reads the system clipboard. A first paste falls back to the kill ring head when the clipboard is empty or unavailable. Once a press has read the clipboard, a further `p` or `P` pastes nothing, and says why, if the clipboard is then empty or unavailable.

Only your own edits count, in any buffer. Changes made in the meantime by a language server, a `|` finishing in the background, or a plugin's timer or hook do not.

Since `y` writes to both the clipboard and the kill ring, `y` then `p` pastes what you just yanked. Yanking to an explicit register instead (`"0y`) leaves both the clipboard and this "what should a bare `p` read" tracking untouched: a following bare `p` behaves as if the `"0y` hadn't happened. Use `"0p` to read the register back.

`[` and `]` only work inside a **paste session**, opened by a preceding `p` or `P`. Each cycle replaces the previous paste, and the whole session is a single undo step. After a paste from the clipboard or a register, `[` skips the kill-ring entry holding the same text, such as the one a `y` just put in both. Consecutive `p` presses append copies, each its own undo step, for the same reason pasting over a matching selection does: pasting the same text again lands next to it, not over it.

Two editor commands, `paste-after`/`paste-before`, exist alongside `p`/`P` with no key bound by default. They always read the kill-ring head, with no clipboard fallback, and always replace a real selection outright, with no same-text check. To stack a copy with them, collapse the selection (`;`) before pasting. [GUI-style paste](#gui-style-paste-bundled-plugin) below is a plugin built on them.

## Clipboard over SSH

In an SSH session, or when no clipboard server is available (a Linux machine without X11 or Wayland), HUME asks your terminal to set the clipboard instead (OSC 52). `y` then puts the text on the clipboard of the machine you are sitting at.

- Your terminal must allow programs to set the clipboard. Some, iTerm2 for one, ask for this in their settings.
- Inside `tmux`, add `set -g set-clipboard on` to your `tmux.conf`.
- `p` pastes what HUME last yanked. Terminals don't let programs read the clipboard back, so text copied in another application comes in through the terminal's own paste shortcut.

## Pasting from the terminal

Text pasted from outside HUME (your system clipboard via the terminal's paste shortcut, a mouse paste, or a paste from `tmux`/`screen`) lands in one step, however long it is.

- In Insert mode, the text is inserted at the cursor. Auto-pairing doesn't run on pasted text, so pasted brackets and quotes are never doubled.
- In Normal or Extend mode, a real selection is replaced; on a bare cursor the text is inserted in front of it.
- On the command prompt, the Search and Sift prompts, and picker query fields, line breaks become spaces and a trailing line break is dropped, since those fields are single-line.

## Whitespace and the kill ring

When the kill-ring head is pure whitespace (any whitespace characters, such as spaces, tabs and line breaks, and nothing else), the next delete, change, or yank overwrites that slot instead of taking a fresh one. This keeps the ring free of entries you'd never want to cycle back to. To keep whitespace for later, yank it into a numbered register (`"0`–`"9`).

The ring never holds two identical entries, meaning the same text with the same shape (whole lines or inline). Deleting, changing, or yanking text already in the ring moves that entry back to the front instead of adding a duplicate, so cycling with `[`/`]` never shows the same entry twice.

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

Two further registers exist but can't be named through the `"` prefix:

| Register | How it's used |
|----------|---------------|
| `q` | Default macro register: written by `Q` recording, read by `q` replay |
| `s` | Search register: holds the last search pattern; written by `/`, `?`, `*`, and reused when you repeat the search |

## Inserting a register while typing

In Insert mode and on the command line, `Ctrl-r` followed by a register name inserts that register's text.

- A register yanked from several selections gives each cursor its own selection's text when there are as many cursors as selections, as `p` does, and all of it joined otherwise. The command line gets all of it joined.
- Two other keys insert paths relative to the working directory: `f` the current file's, and `p` its directory's. The directory path ends in a separator so you can type a file name after it (`./` for a file directly in the working directory). A file outside the working directory gets its full path.
- Any other key inserts nothing and reports that it names no register. `Esc` cancels.

## GUI-style paste (bundled plugin)

To keep the clipboard and the kill ring on separate keys instead of letting `p` choose, load `core:classic-paste`. It rebinds `p` / `P` to always paste from the kill ring, and binds `Ctrl-v` / `Ctrl-Shift-v` to always paste from the system clipboard (`Ctrl-Shift-v` needs the kitty protocol).

```scheme
(load-plugin! "core:classic-paste")
```
