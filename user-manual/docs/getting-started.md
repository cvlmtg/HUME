# Getting Started

Modal editing has a reputation for a steep first hour. It doesn't need one: the ideas below take about five minutes, and they cover most of what you'll do all day.

## First launch

Launch HUME on a file:

```sh
hume path/to/file.txt
```

Or run `hume` with no arguments to open a scratch buffer. The current mode is shown on the right side of the status bar.

## The first minute

1. **Learn the keys:** type `:tutor` and press `Enter` to open the interactive tutorial. The tutorial is an editable copy of the bundled `tutor.rst` — feel free to experiment in it. Re-running `:tutor` later in the same session switches back to your existing buffer, preserving any edits; after `:bd!` it opens a fresh copy.
2. **Where am I?** The mode label (`NOR`, `INS`, `EXT`, …) sits on the right of the status bar; the file path and cursor position sit on the left.
3. **Quit:** `:q` closes the current buffer, and quits HUME once it's the last one open. `:q!` discards unsaved changes. `:wq` saves and quits. `:qa` quits everything.
4. **Turn on the extras (optional):** Out of the box HUME is a lean editor — language-server smarts, fuzzy file and buffer pickers, and the plugin manager stay off until you ask for them. Copy the bundled starter config — `share/hume/init.scm.example` in the macOS/Linux release archive, `runtime/init.scm.example` on Windows or in a source checkout — to `~/.config/hume/init.scm` (`%APPDATA%\hume\init.scm` on Windows) and they're all on next launch. See [Configuration](configuration.md#example-init-scm) for what each line does.

## The modal idea

HUME has distinct modes. Every key you press is interpreted in the context of the current mode — there is no "always-on" Insert mode. This lets the same keys serve as both navigation and commands.

### How is this different from a normal editor?

In a conventional editor every letter you type lands in the document, so actions have to be chorded shortcuts — `Ctrl+C`, `Cmd+V`, `Ctrl+F` — and many moves need the mouse.

Modal editors invert that. In Normal mode letters are *commands*: `d` deletes, `w` jumps a word, `x` selects a line, and nothing you type reaches the buffer. To write prose you switch to Insert mode, type, and press `Esc` to come back. The payoff is that almost every edit is a plain keystroke — no modifiers, no mouse.

## The three main modes

The three modes you will use most:

| Mode | How to enter | What it does |
|------|-------------|--------------|
| Normal | `Esc` from anywhere | Navigate and issue commands |
| Insert | `i`, `a`, `o`, `c` from Normal mode | Type text |
| Extend | `e` from Normal mode | Grow and shrink selections |

You spend most of your time in Normal. Drop into Insert only to type, then `Esc` back. See [Modes](modes.md) for the full list, including command-line, search, and select modes.

## Motions

Motions move the cursor or change the current selection — `w` to the next word, `f` followed by a character to jump to it on the line, `g g` to the first line. Motions are how you navigate, and in Extend mode they are also how you grow or shrink a selection.

See [Moving Around](moving-around.md) for the full set.

## Selection-first editing

HUME follows the **select-then-act** model. You first say *what* you want to act on — a word, a line, a paragraph, a pair of quotes — and then say *what to do with it*: `d` to delete, `c` to change, `y` to yank. There is no cursor-without-selection: what looks like a single-character cursor is just a one-character selection, and every editing command operates on the current selection.

This is the opposite of the more familiar action-then-target order (press `d`, then say what to delete). Selecting first means you always see exactly what an edit will touch before it happens.

See [Selections](selections.md) for how to build and shape selections.

## Multiple selections

HUME can hold several selections at once, and any editing command acts on all of them simultaneously. The classic example is renaming: select every occurrence of a word, press `c`, and type the new name once — every selected instance changes together.

See [Selections](selections.md) for how to create and manage multiple selections.

## Next steps

- [Modes](modes.md) — full description of each mode
- [Moving Around](moving-around.md) — navigating your file
- [Editing](editing.md) — edit your file
- [Copy & Paste](copy-and-paste.md) — yank, paste, and the kill ring
- [Selections](selections.md) — selecting the text
- [Files & Buffers](files-and-buffers.md) — opening, saving, and quitting
- [Key Reference](key-reference.md) — every key by mode
