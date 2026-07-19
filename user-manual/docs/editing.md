# Editing

Say what you mean, then say what to do with it. Because the selection always comes first, you can see exactly what an edit will touch before it happens — and the same handful of action keys work on a character, a word, a block, or forty places at once.

## Inserting text

| Key | Where insertion begins |
|-----|----------------------|
| `i` | Before the selection |
| `a` | After the selection |
| `I` | First non-blank character on line |
| `A` | End of line |
| `o` | New line below |
| `O` | New line above |

Press `Esc` or `Ctrl+c` to return to Normal mode. `Ctrl+w` deletes the word before the cursor while you type.

## Deleting and changing text

| Key | Effect |
|-----|--------|
| `d` | Delete selection (pushed onto the kill ring) |
| `c` | Delete selection content and enter Insert mode (one undo group). A trailing newline is kept — `c` on a line rewrites its content without removing the line itself. |

Use `x` to select the current line first if you want a line-wise delete (`x` then `d`).

`c` keeps the selection on the text you changed: leaving Insert mode selects your replacement instead of leaving a plain cursor, so you can immediately act on it again — delete it, surround it, search for it. Pressing `Esc` without typing anything leaves the cursor where the change began. Disable this with the `select-changed-text` option (see [Configuration](configuration.md)).

Whichever way you entered Insert mode, `m i i` recovers what you last typed after `Esc` — see [Text objects](selections.md#text-objects).

## Replacing text

| Key | Effect |
|-----|--------|
| `r` + char | Replace every selected character with the typed character |

Line endings inside the selection are left alone, so replacing across several lines won't collapse them into one. On a single character, `r` is delimiter-aware: replacing one half of a bracket or quote pair updates its partner too.

## Changing case

| Key | Effect |
|-----|--------|
| `g u` | Lowercase the selection |
| `g U` | Uppercase the selection |
| `g C` | Capitalize the selection |

## Lines and alignment

| Key | Effect |
|-----|--------|
| `J` | Join the selected lines into one |
| `&` | Align selections into a column |

`J` replaces each line break with a single space and drops the next line's indentation, so joining wrapped code or prose doesn't leave a gap in the middle. When the next line is empty or only whitespace, the lines are joined with no space at all. Joining several lines at once leaves one cursor on each inserted space, ready to act on.

`&` lines up your selections using the primary selection's line as the starting point. Spaces are inserted at the left edge of each selection to reach that column, and if some other line needs more room, the column widens for everybody — which means the primary selection can shift right too. Multi-line selections are left alone. Where a selection sits too far right already, the run of spaces or tabs immediately to its left is squeezed down (never below one).

You can align to the left or the right depending on which end of the selection is the anchor; `Ctrl+e` swaps anchor and head. See [Selections](selections.md#flipping-and-collapsing-the-selection).

## Copying and pasting

HUME has two paste sources: the system clipboard and a kill ring that remembers the last 10 things you deleted or yanked.

| Key | Effect |
|-----|--------|
| `y` | Yank (copy) selection — writes to the system clipboard **and** pushes onto the kill ring |
| `p` | Smart-paste after the selection |
| `P` | Smart-paste before the selection |
| `[` | Cycle one step older in the kill ring and re-paste |
| `]` | Cycle one step newer in the kill ring and re-paste |

With a real selection (more than a single character), `p` and `P` both **replace** it — "after" and "before" only apply when the selection is a bare cursor. The replaced text is thrown away rather than pushed onto the kill ring.

### Smart-p paste

`p` and `P` decide what to paste based on the last command:

- **After `d` or `c`** — reads the kill ring head (the most recently killed or changed text).
- **After a paste-family command** (`p`, `P`, `[`, `]`) — re-pastes the same text again, appending another copy onto the previous paste.
- **Otherwise** (including after `y`) — reads the system clipboard, falling back to the kill ring head when the clipboard is empty or unavailable.

Since `y` writes to both the clipboard and the kill ring, `y` then `p` pastes what you just yanked. The exception is yanking to an explicit register (`"0y`): that leaves the clipboard untouched, so a following bare `p` pastes whatever was in the clipboard before. Use `"0p` to read the register back.

`[` and `]` only work inside a **paste session** — one opened by a preceding `p` or `P`. Each cycle replaces the previous paste, and the whole session records as a single undo step. Consecutive `p` presses append copies, each starting a new session and a separate undo step.

### Pasting from the terminal

Pasting text from outside HUME — your system clipboard via the terminal's own paste shortcut, a mouse paste, or a paste from `tmux`/`screen` — lands in one step, however long the pasted text is.

- In Insert mode, the text is inserted at the cursor. Auto-pairing does not run on pasted text, so pasted brackets and quotes are never doubled up.
- In Normal or Extend mode, a real selection is replaced; on a bare cursor the text is inserted in front of it.
- On the command line and in search/select prompts, line breaks in the pasted text become spaces, since those fields are single-line.

### Whitespace and the kill ring

When the current kill-ring head is a pure-whitespace entry (only spaces, tabs, and/or newlines), the next delete, change, or yank overwrites that slot in place instead of taking a fresh one. This stops the ring filling up with entries you'd never want to cycle back to. To keep whitespace durably, yank it into a numbered register (`"0`–`"9`).

### Register prefix (`"`)

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

## Undo and redo

| Key | Effect |
|-----|--------|
| `u` | Undo (accepts a count — `5u` undoes five steps) |
| `U` / `Ctrl+r` | Redo |

Undo history is a tree rather than a straight line, so redoing after new edits follows the most recent branch. The history lives in memory only and starts fresh each time you open a file.

## Repeat

| Key | Effect |
|-----|--------|
| `.` | Repeat the last editing command |

Dot-repeat replays the most recent insert session or editing command — delete, change, paste and so on, but not `y`. Give it a count to override the original: if `3w` set up the first change, `5.` repeats it over five words instead.

## Macros

Macros record and replay sequences of keys and are stored in registers. Register `q` is the default.

| Key | Effect |
|-----|--------|
| `Q Q` or `Q q` | Start recording into the default register `q` |
| `Q <0-9>` | Start recording into a numbered register |
| `Q` (while recording) | Stop recording |
| `q q` | Replay register `q` |
| `q <0-9>` | Replay a numbered register |
| `<count> q q` | Replay register `q` `<count>` times |

Recording is ignored in read-only buffers, and while a macro is already recording or replaying — so a macro can't record itself or nest.

## Numeric count

Prefix a command with digits to repeat it. The first digit must be `1`–`9`; `0` counts only once a count is already under way. So `12w` moves forward 12 words and `10j` moves down 10 lines.

| Example | Effect |
|---------|--------|
| `3w` | Move forward 3 words |
| `5j` | Move down 5 lines |
| `12w` | Move forward 12 words |
| `10j` | Move down 10 lines |

`0` on its own does nothing by default — `g h` goes to the start of the line. If you want vim's `0`, the `core:vim-keybind` plugin binds it (see [Core Plugins](core-plugins.md#core-vim-keybind)).

## Surround

HUME's surround commands select or wrap delimiter pairs using the `m` prefix — see [Selections](selections.md) for the full list of `m i`/`m a` text objects.

| Key | Effect |
|-----|--------|
| `m s` + char | Select the surrounding delimiter pair (both ends) |
| `m w` + char | Wrap each selection with a delimiter pair |

To delete or replace a pair, select it with `m s` and then act: `m s (` then `d` deletes the parentheses, `r` replaces them.

## Helix-style surround (bundled plugin)

If you prefer Helix's dedicated surround keys, a bundled plugin provides them:

| Key | Effect |
|-----|--------|
| `m s` + char | Wrap (add surround) |
| `m d` + char | Delete the surrounding pair |
| `m r` + char + new | Replace the surrounding pair with a new delimiter |

Note that this moves wrapping onto `m s`: it takes over the default `m s` (select the pair) and removes `m w`.

```scheme
(load-plugin "core:helix-surround")
```

## GUI-style paste (bundled plugin)

If you'd rather keep the clipboard and the kill ring on separate keys instead of letting `p` choose, load `core:classic-paste`. It puts the kill ring on `p` / `P` and the system clipboard on `Ctrl+V` / `Ctrl+Shift+V` (the latter needs the kitty protocol).

```scheme
(load-plugin "core:classic-paste")
```
