# Modes

One keyboard, several meanings. `d` deletes only in the mode where `d` means delete, so HUME can put a whole editor's worth of commands on plain letters, with no modifier gymnastics.

The status bar shows the active mode on its right. `Esc` always takes you back to Normal.

## Normal mode

Normal mode is the default. Its keys are commands: they move the cursor, modify text, or switch to another mode. Nothing you type appears in the buffer.

**Enter:** at startup, or `Esc` from any mode

**Exit:** enter another mode (`i`, `e`, `:`, `/`, `s`, …)

## Insert mode

In Insert mode, what you type goes straight into the buffer. The cursor becomes a bar while you insert.

**Enter:** `i`, `a`, `I`, `A`, `o`, `O`, or `c`. See [Editing](editing.md) for what each key does.

**Exit:** `Esc` or `Ctrl-c`

## Extend mode

Extend mode works like Normal mode, but every motion *extends* the current selection instead of replacing it. Use it to build a multi-character or multi-line selection before acting on it. Moving back toward where you started shrinks the selection.

**Enter:** `e` (toggles; status bar shows `EXT`)

**Exit:** `Esc`, `e` again, or `;` (collapse to head) / `Ctrl-;` (collapse to anchor, kitty only)

## Command mode

Command mode opens a prompt at the bottom of the screen where you type a command name. Keys you bind in `'command` mode work here too; see [Keys in the command line](configuration.md#keys-in-the-command-line).

See [Command mode](command-mode.md) for a full list of commands.

**Enter:** `:`

**Exit:** `Enter` runs the command, `Esc` or `Ctrl-c` cancels

## Search mode

Search mode opens a prompt for a search pattern. `n` / `N` cycle through matches afterward.

**Enter:** `/` (forward) or `?` (backward)

**Exit:** `Enter` jumps to the first match, `Esc` or `Ctrl-c` cancels

## Sift mode

Sift mode opens a regex prompt (`⫽`) that narrows each selection to its matching sub-ranges. Use it to break a selection into individual tokens.

**Enter:** `s`

**Exit:** `Enter` narrows the selections, `Esc` or `Ctrl-c` cancels

## Multi-key sequences

Some Normal-mode keys wait for a second key before doing anything. Either they open a family of related commands, or they take a single character as their argument.

### Prefixes

| Prefix | Keys | Purpose |
|--------|------|---------|
| Goto | `g` + key | Jump to a position in the buffer, or to the next/previous instance of a structural kind. See [Default Keys](default-keys.md) |
| `G` | `G` + key | Case transforms, and other non-goto commands Vim puts under `g`. See [Default Keys](default-keys.md) |
| Match | `m` + key | Select text objects and surrounding delimiters. See [Selections](selections.md) |
| Match WORD | `M M` | Select the WORD under the cursor. See [Selections](selections.md) |
| View | `z` + key | Scroll the view to a position, or open a fuzzy picker. See [Moving Around](moving-around.md) and [Fuzzy Finder](pickers.md) |
| Pane | `Ctrl-p` + key | Move focus between panes. See [Default Keys](default-keys.md) |
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

