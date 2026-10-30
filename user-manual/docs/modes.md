# Modes

One keyboard, several meanings. `d` deletes only in the mode where `d` means delete, so HUME can put a whole editor's worth of commands on plain letters, with no modifier gymnastics.

The statusline shows the active mode on its right. `Esc` always takes you back to Normal.

## Normal mode

Normal mode is the default. Its keys are commands: they move the cursor, modify text, or switch to another mode. Nothing you type appears in the buffer.

**Enter:** at startup, or `Esc` from any mode

**Exit:** enter another mode (`i`, `e`, `:`, `/`, `s`, …)

## Insert mode

In Insert mode, what you type goes straight into the buffer. The cursor becomes a bar while you insert.

**Enter:** `i`, `a`, `I`, `A`, `o`, `O`, or `c`. See [Editing](editing.md) for what each key does.

**Exit:** `Esc` or `Ctrl-c`

## Extend mode

Extend mode works like Normal mode, but every motion *extends* the current selection instead of replacing it, and moving back toward where you started shrinks it. See [Extend mode](selections.md#extend-mode).

**Enter:** `e` (toggles; the statusline shows `EXT`)

**Exit:** `Esc`, `e` again, or `;` (collapse to head) / `Ctrl-;` (collapse to anchor, kitty only)

## Command mode

Command mode opens the `:` prompt at the bottom of the screen, where you type a command such as `:w`. See [Command mode](command-mode.md) for the commands, and [Keys in the command line](configuration.md#keys-in-the-command-line) to bind keys there.

**Enter:** `:`

**Exit:** `Enter` runs the command, `Esc` or `Ctrl-c` cancels

## Search mode

Search mode opens a prompt for a search pattern. `n` / `N` cycle through matches afterward.

**Enter:** `/` (forward) or `?` (backward)

**Exit:** `Enter` jumps to the first match, `Esc` or `Ctrl-c` cancels

## Sift mode

Sift mode opens a regex prompt (`⫽`) that narrows each selection to the parts matching the pattern, for example to break a line into words. See [Sift within](selections.md#sift-within-s).

**Enter:** `s`

**Exit:** `Enter` narrows the selections, `Esc` or `Ctrl-c` cancels

## Multi-key sequences

Some Normal-mode keys wait for a second key before doing anything. Either they open a family of related commands, or they take a single character as their argument.

### Prefixes

| Prefix | Keys | Purpose |
|--------|------|---------|
| Goto | `g` + key | Jump to a position in the buffer, or to the next/previous instance of a structural kind. See [Default Keys](default-keys.md) |
| `G` | `G` + key | Change the selected text: case, comments, number sequences. See [Default Keys](default-keys.md) |
| Match | `m` + key | Select text objects, or select and wrap surrounding delimiters. See [Text objects](selections.md#text-objects) and [Surround](editing.md#surround) |
| Match WORD | `M M` | Select the WORD under the cursor. See [Selections](selections.md) |
| View | `z` + key | Scroll the view to a position, or open a fuzzy picker. See [Moving Around](moving-around.md) and [Fuzzy Finder](pickers.md) |
| Pane | `Ctrl-p` + key | Manage panes and tabs: move focus, split, close, rotate, switch tabs. See [Default Keys](default-keys.md) |
| Register | `"` + char | Target a specific register for yank, paste, or delete. See [Copy & Paste](copy-and-paste.md#register-prefix) |

### Keys that take a character

| Keys | Purpose |
|------|---------|
| `f`, `F`, `t`, `T` + char | Jump to a character on the current line. See [Moving Around](moving-around.md) |
| `r` + char | Replace the selected characters with the typed character. See [Editing](editing.md) |
| `m w` + char | Wrap the selection in that character. See [Editing](editing.md#surround) |

### Count prefix

| Prefix | Purpose |
|--------|---------|
| `1`–`9` then `[0-9]*` | Repeat the next command a number of times. See [Editing](editing.md) |

