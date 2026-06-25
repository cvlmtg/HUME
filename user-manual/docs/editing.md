# Editing

HUME follows a **select-then-act** model. You first select the text you want to change, then apply an action to it. Most editing commands operate on the current selection.

## Inserting text

Enter Insert mode with `i`, `a`, `o`, `O`, `I`, or `A`. See [Modes](modes.md) for what each key does.

## Deleting

| Key | Effect |
|-----|--------|
| `d` | Delete selection (pushed onto the kill ring) |
| `c` | Delete selection content and enter Insert mode (one undo group). A trailing newline is kept — `c` on a line rewrites its content without removing the line itself. |

Use `x` to select the current line first if you want a line-wise delete (`x` then `d`). There is no single-key "delete character" command — select the character (or use `c`) and delete.

## Replacing

| Key | Effect |
|-----|--------|
| `r` + char | Replace every selected character with the typed character |

After pressing `r`, HUME waits for the replacement character. The character under each cursor is replaced with the typed character.

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

- **After `d`, `c`, or a paste/ring command** — reads the kill ring head (the most recently killed or pasted text).
- **Otherwise** — reads the system clipboard.

`[` and `]` cycle within the current **paste session** (opened by a preceding `p`/`P`). Each cycle replaces the previous paste, and the whole session records as a single undo step. Consecutive `p` presses append copies (each starts a new session and a separate undo step).

### Register prefix (`"`)

Prefix a yank, delete, change, or paste with `"` + a register name to target a specific source or destination:

| Example | Effect |
|---------|--------|
| `"cp` | Paste from the system clipboard explicitly |
| `"kp` | Paste from the kill-ring head |
| `"0y` | Yank to named register 0 |
| `"5p` | Paste from named register 5 |
| `"bd` | Delete to the black hole (nothing saved) |

Registers addressable via `"` are: `0`–`9` (named storage, symmetric — `"5y` writes and `"5p` reads the same slot), `k` (kill-ring head), `c` (system clipboard), and `b` (black hole).

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

Prefix a motion with a number `1`–`9` to repeat it that many times:

| Example | Effect |
|---------|--------|
| `3w` | Move forward 3 words |
| `5j` | Move down 5 lines |

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