# Editing

HUME follows a **select-then-act** model. You first select the text you want to change, then apply an action to it. Most editing commands operate on the current selection.

## Inserting text

| Key | Where insertion begins |
|-----|----------------------|
| `i` | Before the selection |
| `a` | After the selection |
| `I` | First non-blank character on line |
| `A` | End of line |
| `o` | New line below |
| `O` | New line above |

Press `Esc` to return to Normal mode.

## Deleting

| Key | Effect |
|-----|--------|
| `d` | Delete selection (pushed onto the kill ring) |
| `c` | Delete selection content and enter Insert mode (one undo group). A trailing newline is kept — `c` on a line rewrites its content without removing the line itself. |

Use `x` to select the current line first if you want a line-wise delete (`x` then `d`).

After changing text with `c`, leaving Insert mode selects the text you just typed, so you can immediately act on it again — delete it, surround it, search for it. Pressing `Esc` without typing anything leaves the cursor where the change began. Disable this with the `select-changed-text` option (see [Configuration](configuration.md)).

## Replacing

| Key | Effect |
|-----|--------|
| `r` + char | Replace every selected character with the typed character |

After pressing `r`, HUME waits for the replacement character. All the characters in the selection are replaced with the typed character.

## Lines and alignment

| Key | Effect |
|-----|--------|
| `J` | Join the current line with the next, replacing the newline with a space; the cursor lands on the inserted space. If the selection spans multiple lines, all are joined into one. |
| `&` | Align all selections to the column of the primary selection's anchor. Spaces are inserted or removed at the left edge of each selection. Multi-line selections are left unchanged. |

## Copying and pasting

HUME has two paste sources: the system clipboard and an internal kill ring.

| Key | Effect |
|-----|--------|
| `y` | Yank (copy) selection — writes to the system clipboard **and** pushes onto the kill ring |
| `p` | Smart-paste after the selection |
| `P` | Smart-paste before the selection |
| `[` | Cycle one step older in the kill ring and re-paste |
| `]` | Cycle one step newer in the kill ring and re-paste |

### Smart-p paste

`p` and `P` decide what to paste based on the last command:

- **After `d` or `c`** — reads the kill ring head (the most recently killed or changed text).
- **After a paste-family command** (`p`, `P`, `[`, `]`) — re-pastes `last_paste` verbatim, appending onto the previous paste.
- **Otherwise** (including after `y`) — reads the system clipboard.

The clipboard rule after `y` deserves a note: `y` writes the yanked text to **both** the system clipboard and the kill ring, so `y` then `p` pastes the clipboard, which is the text you just yanked — the common case behaves as expected. The subtlety is when you yank to an explicit non-default register (e.g. `"0y`): the clipboard is left untouched and a following `p` pastes whatever was previously in the clipboard. To paste from the just-yanked named register, prefix with the register: `"0p`.

`[` and `]` cycle within the current **paste session** (opened by a preceding `p`/`P`). Each cycle replaces the previous paste, and the whole session records as a single undo step. Consecutive `p` presses append copies (each starts a new session and a separate undo step).

### Whitespace and the kill ring

When the current kill-ring head is a pure-whitespace entry (only spaces, tabs, and/or newlines), the next delete, change, or yank overwrites that slot in place instead of taking a fresh one. This stops filling the ring with entries you would never want to cycle back to. The just-killed whitespace remains retrievable until the next capture, so a swap still works; afterwards it is gone. To keep whitespace durably, yank it into a named register (`"0`–`"9`).

### Register prefix (`"`)

Prefix a yank, delete, change, or paste with `"` + a register name to target a specific source or destination:

| Example | Effect |
|---------|--------|
| `"cp` | Paste from the system clipboard explicitly |
| `"kp` | Paste from the kill-ring head |
| `"0y` | Yank to named register 0 |
| `"5p` | Paste from named register 5 |
| `"bd` | Delete to the black hole (nothing saved) |

Four kinds of register are addressable via `"`:

| Register | Contents |
|----------|----------|
| `0`–`9` | Named storage, symmetric — `"5y` writes and `"5p` reads the same slot |
| `k` | Kill-ring head |
| `c` | System clipboard |
| `b` | Black hole — writes are discarded, reads return nothing |

Two further registers exist but cannot be named through the `"` prefix:

| Register | How it's used |
|----------|---------------|
| `q` | Default macro register — written by `Q` recording, read by `q` replay |
| `s` | Search register — holds the last search pattern; written by `/`/`?`/`*`, read by the search system |

## Undo and redo

| Key | Effect |
|-----|--------|
| `u` | Undo |
| `U` / `Ctrl+r` | Redo |

HUME has a full undo tree — branching history is preserved. If you undo several steps and then type, the previous "future" is kept and accessible.

## Repeat

| Key | Effect |
|-----|--------|
| `.` | Repeat the last editing command |

Dot-repeat replays the most recent insert session or editing command (delete, change, etc.). Useful for applying the same change in multiple locations.

## Macros

Macros record and replay sequences of keys and are stored in registers. Register `q` is the default.

| Key | Effect |
|-----|--------|
| `Q Q` | Start recording into the default register `q` |
| `Q <reg>` | Start recording into a named register (`0`–`9`) |
| `Q` (while recording) | Stop recording |
| `q q` | Replay register `q` |
| `q <reg>` | Replay a named register (`0`–`9`) |
| `<count> q q` | Replay register `q` `<count>` times |

## Numeric count

Prefix a motion with one or more digits — the first digit must be `1`–`9`; `0` is a digit only inside an already-started count, otherwise `0` is `goto-line-start`. So `12w` moves forward 12 words and `10j` moves down 10 lines, while `0` alone jumps to the start of the line.

| Example | Effect |
|---------|--------|
| `3w` | Move forward 3 words |
| `5j` | Move down 5 lines |
| `12w` | Move forward 12 words |
| `10j` | Move down 10 lines |

## Surround

HUME's surround commands select or wrap delimiter pairs using the `m` prefix — see [Selections](selections.md) for the full list of `m i`/`m a` text objects and surround commands.

| Key | Effect |
|-----|--------|
| `m s` + char | Select the surrounding delimiter pair (both ends) |
| `m w` + char | Wrap each selection with a delimiter pair |

The `m` key is a prefix — the status bar shows "match" while waiting for the next key.

## Helix-style surround (bundled plugin)

If you prefer Helix-style surround, a built-in plugin overrides the default `m s` binding and adds `m d` / `m r`:

| Key | Effect |
|-----|--------|
| `m s` + char | Wrap (add surround) |
| `m d` + char | Delete the surrounding pair |
| `m r` + char + new | Replace the surrounding pair with a new delimiter |

Load the plugin in your `init.scm`:

```scheme
(load-plugin "core:helix-surround")
```
