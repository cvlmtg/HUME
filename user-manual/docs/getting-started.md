# Getting Started

Modal editing has a reputation for a steep first hour. It needn't have one: the ideas below take about five minutes and cover most of what you'll do all day.

## First launch

Launch HUME on a file:

```sh
hume path/to/file.txt
```

Or run `hume` with no arguments to open a scratch buffer.

## The first minute

1. **Learn the keys:** type `:tutor` and press `Enter` to open the interactive tutorial. It is your own copy, so edit it freely.
2. **Where am I?** The mode label (`NOR`, `INS`, `EXT`, …) sits on the right of the status bar; the file path and cursor position sit on the left.
3. **Quit:** `:q` closes the current buffer, and quits HUME once it's the last one open. `:q!` discards unsaved changes. `:wq` saves and quits. `:qa` quits everything.
4. **Turn on the extras (optional):** HUME starts lean: language-server smarts, fuzzy file and buffer pickers, and the plugin manager stay off until you ask for them. Copy the bundled starter config (`share/hume/runtime/init.scm.example` in the macOS/Linux release archive, `runtime/init.scm.example` on Windows or in a source checkout) to `~/.config/hume/init.scm` (`%APPDATA%\hume\init.scm` on Windows) and they're all on from the next launch. See [Configuration](configuration.md#example-init-scm) for what each plugin does.

## The modal idea

HUME has distinct modes, and the current mode decides what each key you press does. There is no "always-on" Insert mode, so the same keys can serve as both navigation and commands.

### How is this different from a normal editor?

In a conventional editor every letter you type lands in the document, so actions have to be chorded shortcuts (`Ctrl-c`, `Cmd-v`, `Ctrl-f`), and many moves need the mouse.

Modal editors invert that. In Normal mode letters are *commands*: `d` deletes, `w` jumps a word, `x` selects a line, and nothing you type reaches the buffer. To write text, switch to Insert mode, type, and press `Esc` to come back. The payoff: almost every edit is a plain keystroke, with no modifiers and no mouse.

## The three main modes

The modes you'll use most:

| Mode | How to enter | What it does |
|------|-------------|--------------|
| Normal | `Esc` from anywhere | Navigate and issue commands |
| Insert | `i`, `a`, `o`, `c` from Normal mode | Type text |
| Extend | `e` from Normal mode | Grow and shrink selections |

You spend most of your time in Normal, dropping into Insert only to type. See [Modes](modes.md) for the full list, including command-line, search, and sift modes.

## Motions

Motions move the cursor or change the current selection: `w` to the next word, `f` followed by a character to jump to it on the line, `g g` to the first line. In Extend mode they also grow or shrink the selection.

See [Moving Around](moving-around.md) for the full set.

## Selection-first editing

HUME follows the **select-then-act** model. First say *what* to act on (a word, a line, a paragraph, a pair of quotes), then *what to do with it*: `d` to delete, `c` to change, `y` to yank. There is no cursor without a selection. What looks like a single-character cursor is a one-character selection, and every editing command acts on the current selection.

This reverses the more familiar action-then-target order (press `d`, then say what to delete). Selecting first means you always see what an edit will touch before it happens.

See [Selections](selections.md) for how to build and shape selections.

## Multiple selections

HUME can hold several selections at once, and any editing command acts on all of them together. The classic example is renaming: select every occurrence of a word, press `c`, and type the new name once to change them all.

See [Selections](selections.md) for how to create and manage multiple selections.

## Try it: your first edit

Put all of the above together on a practice file. Run `hume practice.txt`; the file is created when you save it.

| Step | Keys | What happens |
|------|------|--------------|
| Type three lines | `i`, then type `apples are red`, `Enter`, `bananas are yellow`, `Enter`, `cherries are red`, then `Esc` | You're back in Normal mode, and what you typed stays selected |
| Go to the end of the first line | `g g` `g l` | The cursor sits on the `d` of `red` |
| Change `red` to `green` | `m i w` `c` `green` `Esc` | `m i w` selects the word, `c` deletes it and starts Insert mode |
| Delete the second line | `j` `x` `d` | `x` selects the whole line, `d` deletes it |
| Change every `are` to `were` | `%` `s` `are` `Enter` `c` `were` `Esc` | `%` selects the whole file, `s` keeps only the matches: one selection per `are`, all changed at once |
| Undo, then redo | `u` `U` | `u` brings `are` back, `U` changes it to `were` again |
| Save and quit | `:wq` `Enter` | |

The file now reads:

```
apples were green
cherries were red
```

## Next steps

- [Modes](modes.md): full description of each mode
- [Moving Around](moving-around.md): navigating your file
- [Editing](editing.md): edit your file
- [Copy & Paste](copy-and-paste.md): yank, paste, and the kill ring
- [Selections](selections.md): selecting the text
- [Files & Buffers](files-and-buffers.md): opening, saving, and quitting
- [Default Keys](default-keys.md): every key by mode
