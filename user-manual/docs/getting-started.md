# Getting Started

## First launch

Launch HUME on a file:

```
hume path/to/file.txt
```

Or run `hume` with no arguments to open a scratch buffer. The current mode is shown on the left side of the status bar.

For a hands-on introduction, type `:tutor` to launch the interactive tutorial.

## The modal idea

HUME has distinct modes. Every key you press is interpreted in the context of the current mode — there is no "always-on" Insert mode. This lets the same keys serve as both navigation and commands.

### How is this different from a normal editor?

In a conventional editor, every letter you type lands in the document, and most actions are chorded shortcuts held with `Ctrl` or `Cmd` (`Ctrl+C`, `Cmd+V`, `Ctrl+F`, …). Editing is a constant dance between the main keys and modifier keys, and many moves require the mouse.

Modal editors invert this. In Normal mode, letters are *commands* — `d` deletes, `w` jumps a word, `x` selects a line. Nothing you type reaches the buffer. To type prose you switch to Insert mode, type your text, and press `Esc` to return to Normal. Because keys are contextual, the same letter means different things in different modes, and the vast majority of edits need no modifiers and no mouse.

## The three main modes

The three modes you will use most:

| Mode | How to enter | What it does |
|------|-------------|--------------|
| Normal | `Esc` from anywhere | Navigate and issue commands |
| Insert | `i`, `a`, `o`, `c` from Normal mode | Type text |
| Extend | `e` from Normal mode | Grow selections |

You spend most of your time in Normal. Drop into Insert only to type, then `Esc` back. See [Modes](modes.md) for the full list, including command-line, search, and select modes.

## Motions

Motions move the cursor or change the current selection — `w` to the next word, `f` followed by a character to jump to it on the line, `g g` to the first line. Motions are how you navigate, and in Extend mode they are also how you grow a selection.

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
- [Selections](selections.md) — selecting the text
- [Files & Buffers](files-and-buffers.md) — opening, saving, and quitting
- [Key Reference](key-reference.md) — every key by mode
