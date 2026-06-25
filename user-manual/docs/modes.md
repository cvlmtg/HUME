# Modes

HUME is a modal editor. The meaning of every key depends on the current mode. The active mode is shown in the status bar (mode label on the right).

## Normal mode

Normal mode is the default. You land here at startup and return here with `Esc`. In Normal mode, keys are commands — they move the cursor, modify text, or switch to another mode. Nothing you type appears in the buffer.

**Enter from:** any mode via `Esc`

## Insert mode

Insert mode lets you type text directly into the buffer. The cursor changes to a bar to indicate you are inserting.

**Enter from Normal:**

| Key | Where insertion begins |
|-----|----------------------|
| `i` | Before the selection |
| `a` | After the selection |
| `o` | New line below |
| `O` | New line above |
| `I` | Start of line |
| `A` | End of line |

Press `Esc` to return to Normal mode.

## Extend mode

Extend mode works like Normal mode, but every motion *extends* the current selection instead of replacing it. Use it to build up a multi-character or multi-line selection before acting on it.

**Enter:** `e` (toggles; status bar shows `EXT`)

**Exit:** `Esc`, `e` again, or `;` (collapse to head) / `Ctrl+;` (collapse to anchor)

## Command mode

Invoked with `:`, command mode opens the command line at the bottom of the screen. Type a command name and press `Enter` to execute. Press `Esc` to dismiss.

See [Commands](commands.md) for a full list of commands.

## Search mode

Invoked with `/` (forward) or `?` (backward). Type a pattern and press `Enter` to jump to the first match. `n` / `N` cycle through matches afterward.

## Select mode

Invoked with `s` in Normal mode. Select mode opens a regex prompt (`⫽`). Enter a pattern and press `Enter` to filter each existing selection, keeping only sub-ranges that match. Use it to split a selection into individual tokens.

**Enter from Normal:** `s`

## Multi-key sequences

Some Normal-mode keys take a second input. They fall into two groups: **prefix states** (a transient mode that waits for a follow-up key) and **char-argument motions** (a motion that consumes one typed character).

### Prefix states

| Prefix | Keys | Purpose |
|--------|------|---------|
| Goto | `g` + key | Jump to a line or column position — see [Moving Around](moving-around.md) |
| Match | `m` + key | Select text objects and surrounding delimiters — see [Selections](selections.md) |
| View | `z` + key | Scroll the view to a position — see [Moving Around](moving-around.md) |
| Pane | `Ctrl+p` + key | Move focus between panes — see [Key Reference](key-reference.md) |
| Register | `"` + char | Target a specific register for yank, paste, or delete — see [Editing](editing.md) |

### Char-argument motions

| Keys | Purpose |
|------|---------|
| `f`, `F`, `t`, `T` + char | Search for a character on the current line — see [Moving Around](moving-around.md) |
| `r` + char | Replace the selected characters with the typed character — see [Editing](editing.md) |

### Count prefix

| Prefix | Purpose |
|--------|---------|
| `1`–`9` then `[0-9]*` | Repeat the next command a number of times — see [Editing](editing.md) |

