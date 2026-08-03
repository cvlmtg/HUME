# Copy & Paste

HUME has two paste sources: the system clipboard and a kill ring that remembers the last 10 things you deleted or yanked.

| Key | Effect |
|-----|--------|
| `y` | Yank (copy) selection — writes to the system clipboard **and** pushes onto the kill ring |
| `p` | Smart-paste after the selection |
| `P` | Smart-paste before the selection |
| `[` | Cycle one step older in the kill ring and re-paste |
| `]` | Cycle one step newer in the kill ring and re-paste |

With a real selection (more than a single character), `p` and `P` **replace** it — unless what you're pasting is already exactly the selected text, in which case they paste right alongside it instead, so pressing `p` again after a paste adds another copy rather than overwriting the first one. "After" and "before" apply on a bare cursor, or once a matching selection has collapsed this way. The replaced text (when something is replaced) is thrown away rather than pushed onto the kill ring.

## Smart-p paste

`p` and `P` decide what to paste based on whether the buffer has changed since the ring last did:

- **Nothing edited since the last `d`, `c`, or `y`** — reads the kill ring head (the most recently killed, changed, or yanked text).
- **Something edited since** — reads the system clipboard, falling back to the kill ring head when the clipboard is empty or unavailable.

Since `y` writes to both the clipboard and the kill ring, `y` then `p` pastes what you just yanked. Yanking to an explicit register instead (`"0y`) leaves both the clipboard and this "what should a bare `p` read" tracking untouched — a following bare `p` behaves exactly as it would have if the `"0y` hadn't happened. Use `"0p` to read the register back.

`[` and `]` only work inside a **paste session** — one opened by a preceding `p` or `P`. Each cycle replaces the previous paste, and the whole session records as a single undo step. Consecutive `p` presses append copies (each its own separate undo step) for the same reason pasting over a matching selection does: pasting the same text again lands next to it, not over it.

Two plain commands, `paste-after`/`paste-before`, exist alongside `p`/`P` with no key bound by default: they always read the kill-ring head, with no clipboard fallback, and always replace a real selection outright — no same-text check. To stack a copy with plain paste, collapse the selection (`;`) before pasting. See [GUI-style paste](#gui-style-paste-bundled-plugin) below for a plugin built on them.

## Pasting from the terminal

Pasting text from outside HUME — your system clipboard via the terminal's own paste shortcut, a mouse paste, or a paste from `tmux`/`screen` — lands in one step, however long the pasted text is.

- In Insert mode, the text is inserted at the cursor. Auto-pairing does not run on pasted text, so pasted brackets and quotes are never doubled up.
- In Normal or Extend mode, a real selection is replaced; on a bare cursor the text is inserted in front of it.
- On the command line and in search/select prompts, line breaks in the pasted text become spaces, since those fields are single-line.

## Whitespace and the kill ring

When the current kill-ring head is a pure-whitespace entry (only spaces, tabs, and/or newlines), the next delete, change, or yank overwrites that slot in place instead of taking a fresh one. This stops the ring filling up with entries you'd never want to cycle back to. To keep whitespace durably, yank it into a numbered register (`"0`–`"9`).

The ring never holds two identical entries. Deleting, changing, or yanking text that's already in the ring moves that entry back to the front instead of adding a duplicate — so cycling with `[`/`]` never repeats the same text twice.

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
| `0`–`9` | Numbered storage — `"5y` writes and `"5p` reads the same slot |
| `k` | Kill-ring head |
| `c` | System clipboard |
| `b` | Black hole — writes are discarded, reads return nothing |

::: warning Numbered registers are shared with macros
`"3y` and `Q 3` write to the same slot, and the last write wins — recording a macro into `3` overwrites text you stored there, and yanking into `3` destroys the macro. Keep the two uses on separate numbers.
:::

Two further registers exist but cannot be named through the `"` prefix:

| Register | How it's used |
|----------|---------------|
| `q` | Default macro register — written by `Q` recording, read by `q` replay |
| `s` | Search register — holds the last search pattern; written by `/`, `?`, `*`, and reused when you repeat the search |

## GUI-style paste (bundled plugin)

If you'd rather keep the clipboard and the kill ring on separate keys instead of letting `p` choose, load `core:classic-paste`. It rebinds `p` / `P` to always paste from the kill ring, and binds `Ctrl+V` / `Ctrl+Shift+V` to always paste from the system clipboard (the latter needs the kitty protocol).

```scheme
(load-plugin "core:classic-paste")
```
