# Modes

One keyboard, several meanings. Because `d` only deletes when you're in the mode where `d` means delete, HUME can put a whole editor's worth of commands on plain letters — no modifier gymnastics.

The active mode is shown on the right of the status bar. `Esc` always takes you back to Normal.

## Normal mode

Normal mode is the default. You land here at startup and return here with `Esc`. In Normal mode, keys are commands — they move the cursor, modify text, or switch to another mode. Nothing you type appears in the buffer.

**Enter from:** any mode via `Esc`

## Insert mode

Insert mode lets you type text directly into the buffer. The cursor changes to a bar to indicate you are inserting.

**Enter from Normal:**

Enter Insert mode with `i`, `a`, `I`, `A`, `o`, `O`, or `c`. See [Editing](editing.md) for what each key does.

**Exit:** `Esc` or `Ctrl+c`

## Extend mode

Extend mode works like Normal mode, but every motion *extends* the current selection instead of replacing it. Use it to build up a multi-character or multi-line selection before acting on it. Motions also run in reverse: moving back toward where you started shrinks the selection instead of growing it.

**Enter:** `e` (toggles; status bar shows `EXT`)

**Exit:** `Esc`, `e` again, or `;` (collapse to head) / `Ctrl+;` (collapse to anchor, kitty only)

## Command mode

Invoked with `:`, command mode opens the command line at the bottom of the screen. Type a command name and press `Enter` to execute. Press `Esc` to dismiss.

See [Commands](commands.md) for a full list of commands.

## Search mode

Invoked with `/` (forward) or `?` (backward). Type a pattern and press `Enter` to jump to the first match. `n` / `N` cycle through matches afterward.

## Select mode

Invoked with `s` in Normal mode. Select mode opens a regex prompt (`⫽`). Enter a pattern and press `Enter` to filter each existing selection, keeping only sub-ranges that match. Use it to split a selection into individual tokens.

**Enter from Normal:** `s`

## Multi-key sequences

Some Normal-mode keys wait for a second key before doing anything. Either they open a small family of related commands, or they take a single character as their argument.

### Prefixes

| Prefix | Keys | Purpose |
|--------|------|---------|
| Goto | `g` + key | Jump to a line or column position — see [Moving Around](moving-around.md) |
| Match | `m` + key | Select text objects and surrounding delimiters — see [Selections](selections.md) |
| Match WORD | `M M` | Select the WORD under the cursor — see [Selections](selections.md) |
| View | `z` + key | Scroll the view to a position — see [Moving Around](moving-around.md) |
| Pane | `Ctrl+p` + key | Move focus between panes — see [Key Reference](key-reference.md) |
| Register | `"` + char | Target a specific register for yank, paste, or delete — see [Editing](editing.md) |

### Keys that take a character

| Keys | Purpose |
|------|---------|
| `f`, `F`, `t`, `T` + char | Jump to a character on the current line — see [Moving Around](moving-around.md) |
| `r` + char | Replace the selected characters with the typed character — see [Editing](editing.md) |
| `m w` + char | Wrap the selection in that character — see [Selections](selections.md) |

### Count prefix

| Prefix | Purpose |
|--------|---------|
| `1`–`9` then `[0-9]*` | Repeat the next command a number of times — see [Editing](editing.md) |

