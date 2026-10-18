# Editing

Say what you mean, then say what to do with it. Because the selection always comes first, you can see exactly what an edit will touch before it happens, and the same handful of action keys work on a character, a word, a block, or forty places at once.

## Inserting text

| Key | Where insertion begins |
|-----|----------------------|
| `i` | Before the selection |
| `a` | After the selection |
| `I` | First non-blank character on line |
| `A` | End of line |
| `o` | New line below |
| `O` | New line above |

Press `Esc` or `Ctrl-c` to return to Normal mode. `Ctrl-w` deletes the word before the cursor while you type.

Leaving Insert mode selects the text you just typed, instead of leaving a plain cursor, so you can immediately act on it again: delete it, surround it, search for it. Pressing `Esc` without typing anything leaves the cursor where you started. Disable this with the `select-inserted-text` option (see [Configuration](configuration.md)).

Because of this, `i` re-enters Insert mode *before* the current selection, so typing something, pressing `Esc`, then pressing `i` again continues before what you just typed, not after it. Use `a` to continue typing after it instead.

Whichever way you entered Insert mode, `m i i` recovers what you last typed after `Esc`. See [Text objects](selections.md#text-objects).

## Deleting and changing text

| Key | Effect |
|-----|--------|
| `d` | Delete selection (pushed onto the kill ring) |
| `c` | Delete selection content and enter Insert mode (one undo group). A trailing newline is kept: `c` on a line rewrites its content without removing the line itself. |

Use `x` to select the current line first if you want a line-wise delete (`x` then `d`).

`d` takes everything the selection covers except the buffer's final line break; see [Lines and inline text](copy-and-paste.md#lines-and-inline-text). When several cursors' deletions overlap, such as `Ctrl-w` in Insert mode with cursors in the same word, the overlapping text is removed once.

See [Copy & Paste](copy-and-paste.md) for what the kill ring is and how to paste from it.

## Replacing text

| Key | Effect |
|-----|--------|
| `r` + char | Replace every selected character with the typed character |

Line endings inside the selection are left alone, so replacing across several lines won't collapse them into one. On a single character, `r` is delimiter-aware: replacing one half of a bracket or quote pair updates its partner too.

`Enter` and `Tab` count as the typed character, replacing with a newline or tab.

## Changing case

| Key | Effect |
|-----|--------|
| `G L` | Lowercase the selection |
| `G U` | Uppercase the selection |
| `G C` | Capitalize the selection |

## Numbers and dates

| Key | Effect |
|-----|--------|
| `Ctrl-a` | Add the count (1 without one) to the number or date at each selection |
| `Ctrl-s` | Subtract the count from the number or date at each selection |
| `G A` | Add the count to the first selection's number or date, twice the count to the second's, and so on |
| `G S` | Subtract in the same way as `G A` |

A cursor changes the number or date it is on, or else the first one after it on the same line. A wider selection changes the one at its start, or else the first one inside it. The new value ends up selected (for a date or time, the part that changed), so pressing the key again keeps changing it. Selections with nothing to change are left alone.

`G A` numbers a list: select the lines, split them into one selection each with `S`, and three lines of `0` become `1`, `2` and `3`. Selections with nothing to change don't count towards the sequence.

Numbers:

- A `-` before a digit is a minus sign, unless it follows a letter, digit, `_` or closing bracket: in `x-1`, `1-1` and `f(x)-1` it separates, and only the last `1` changes.
- A number right after a letter, `_` or closing bracket never goes below zero: `item1` minus 3 is `item0`.
- Characters listed in the buffer's `word-chars` option count as letters for both rules (see [Configuration](configuration.md)).
- Hexadecimal (`0x1f`), octal (`0o17`) and binary (`0b101`) numbers change in their own base and never go below zero. Hex letters keep their case.
- Leading zeros keep the width (`007` becomes `008`), and `_` separators are kept (`999_999` becomes `1_000_000`).
- Digits inside a word count: `item1` becomes `item2`.
- A `.` separates two numbers: in `1.2.3`, each part changes on its own.

Dates and times:

- Dates are written `2026-10-18` or `2026/10/18`, optionally followed by a space or `T` and a time such as `14:30` or `14:30:15`. A time can also stand on its own.
- The part the selection starts on changes: with the cursor on the month, the month. A selection that starts before the date changes the day, or the minutes of a time on its own. Several selections on one date each change their own part.
- Days, hours, minutes and seconds carry over: `2026-12-31` plus one day is `2027-01-01`. A month or year change keeps the day valid: `2024-01-31` plus one month is `2024-02-29`. A time on its own wraps around midnight.
- Something that is not a real date, such as `2026-13-45`, changes as separate numbers.

When no selection has anything to change, or a result doesn't fit (past the largest supported number, or a year past 9999), nothing changes and a message says so.

## Lines and alignment

| Key | Effect |
|-----|--------|
| `J` | Join the selected lines into one |
| `&` | Align selections into a column |
| `>` | Indent lines touched by a selection |
| `<` | Unindent lines touched by a selection |

`J` replaces each line break with a single space and drops the next line's indentation, so joining wrapped code or prose doesn't leave a gap in the middle. When the next line is empty or only whitespace, the lines are joined with no space at all. Joining several lines at once leaves one cursor on each inserted space, ready to act on.

`&` lines up your selections using the primary selection's line as the starting point. Spaces are inserted at the left edge of each selection to reach that column, and if some other line needs more room, the column widens for everybody, which means the primary selection can shift right too. Multi-line selections are left alone. Where a selection sits too far right already, the run of spaces or tabs immediately to its left is squeezed down (never below one).

You can align to the left or the right depending on which end of the selection is the anchor; `Ctrl-e` swaps anchor and head. See [Selections](selections.md#flipping-and-collapsing-the-selection).

`>`/`<` shift every line touched by a selection by one indent level, using the buffer's `tab-width` and `tab-style` (see [Configuration](configuration.md)). A prefix count shifts by that many levels at once (`3>`). A blank or whitespace-only line inside the selection is left alone, so it never picks up trailing whitespace. Each touched line's whole indent is re-rendered to the new width in the current `tab-style`, not just prepended to or trimmed from, so `<` immediately after `>` restores the previous indent width exactly (the characters themselves are re-rendered in `tab-style`, so a mixed tabs-and-spaces indent normalizes rather than coming back byte-identical). `<` on an indent narrower than one level flattens it to the left margin instead of going negative, so `>` afterward lands on a full level rather than back where the first `<` started.

## Indentation

Enter, `o` and `O` start the new line at the indent the surrounding code calls for: one level deeper inside a block you just opened, level with the code around it otherwise. With `auto-pairs` on, Enter between a pair of brackets opens a block: the cursor lands on an indented line and the closing bracket moves to the line below.

When the first thing you type on a new line closes a block, such as a closing bracket or keyword, the line moves back to the block's indent. A closing word moves back once it is finished, when you type a space or punctuation after it or press Enter. A line whose indent you changed by hand is left alone.

This uses the language's grammar and its indentation rules, which `:plum-install-grammar <language>` installs. Without them, a new line just copies the indent of the current line, and nothing moves back as you type. Set `smart-indent` to `#f` to turn automatic indentation off: new lines then start at the left margin.

New indentation follows the buffer's `tab-width` and `tab-style`, as `>` and `<` do. To indent with spaces in a language, set its `tab-style` to `soft` (see [Configuration](configuration.md)).

## Comments

| Key | Effect |
|-----|--------|
| `G G` | Comment or uncomment the lines of each selection |

On a cursor, or a selection within one line, `G G` puts a line comment (`//` in Rust) in front of the line's text. In a language with only block comments (`<!-- -->` in HTML), it wraps the line's text instead.

A selection over several lines is wrapped in one block comment, from its first non-blank character to its last. When that text already holds the end of a block comment (`*/` in Rust), wrapping it would end the comment early, so its lines get line comments instead; in a language with only block comments, `G G` leaves the text alone and says why. In a language with only line comments, each line gets one, all at the same column, and blank lines are left alone. `:set prefer-line-comments=true` gives several lines line comments even where the language has block comments.

`G G` on a commented line removes the comment, whichever kind it is: a line comment with the space after it, or a block comment's start and end with the space inside each.

With several selections, the primary selection decides, or the first selection with text when the primary's lines are all blank. When its lines are commented, `G G` uncomments every selection; otherwise it comments every selection, including lines that already have a comment.

The comment follows the language at the cursor: in a Markdown file, a fenced Rust block gets `//` and the text around it gets `<!-- -->`. To change a language's comments, see `configure-language!` in [Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language).

## Undo and redo

| Key | Effect |
|-----|--------|
| `u` | Undo (accepts a count: `5u` undoes five steps) |
| `U` / `Ctrl-r` | Redo (accepts a count: `5U` redoes five steps) |

Undo history is a tree rather than a straight line. Redo follows the branch you last moved along: the newest edit, or the branch you jumped into with [`core:undotree`](core-plugins.md#core-undotree). The history lives in memory only and starts fresh each time you open a file.

## Repeat

| Key | Effect |
|-----|--------|
| `.` | Repeat the last editing command |

`.` repeats the last editing command or insert session: delete, change, paste and so on, but not `y`. Typed keys re-run at the new cursor. A choice you made (the character after `r`, a picker or menu item, a prompt answer) is reused, not asked again. An accepted completion writes the same text at the cursor, without its edits elsewhere, such as an added import. A pick in a drawer (references, the undo tree) is never repeated. For custom Insert-mode bindings, see [Falling back to a key's normal Insert-mode behaviour](configuration.md#falling-back-to-a-keys-normal-insert-mode-behaviour).

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

Recording is ignored in read-only buffers, and while a macro is already recording or replaying, so a macro can't record itself or nest.

While a macro is recording, the default statusline shows `[recording @q]`, with the register being recorded into, on its right. It is the `"MacroRecording"` element; see [Statusline](configuration.md#statusline) to place it in a custom layout.

## Numeric count

Prefix a command with digits to repeat it. The first digit must be `1`–`9`; `0` counts only once a count is already under way. So `12w` moves forward 12 words and `10j` moves down 10 lines. Counts above 10,000 are capped at 10,000.

| Example | Effect |
|---------|--------|
| `3w` | Move forward 3 words |
| `5j` | Move down 5 lines |
| `12w` | Move forward 12 words |
| `10j` | Move down 10 lines |

`0` on its own does nothing by default: `g h` goes to the start of the line. If you want vim's `0`, the `core:vim-keybind` plugin binds it (see [Core Plugins](core-plugins.md#core-vim-keybind)).

## Surround

HUME's surround commands select or wrap delimiter pairs using the `m` prefix. See [Selections](selections.md) for the full list of `m i`/`m a` text objects.

| Key | Effect |
|-----|--------|
| `m s` + char | Select the surrounding delimiter pair (both ends) |
| `m w` + char | Wrap each selection with a delimiter pair |

To delete or replace a pair, select it with `m s` and then act: `m s (` then `d` deletes the parentheses, `r` replaces them.

<div class="key-demo">
<strong>Cursor inside, press <code>m</code> <code>s</code> <code>(</code></strong><br>
call<span class="head">(</span>one, two<span class="head">)</span><br>
<br>
<strong>Press <code>d</code></strong><br>
callone, two<br>
<br>
<strong>...or press <code>r</code> <code>[</code> instead</strong><br>
call[one, two]
</div>

## Helix-style surround (bundled plugin)

If you prefer Helix's dedicated surround keys, a bundled plugin provides them:

| Key | Effect |
|-----|--------|
| `m s` + char | Wrap (add surround) |
| `m d` + char | Delete the surrounding pair |
| `m r` + char + new | Replace the surrounding pair with a new delimiter |

This moves wrapping onto `m s`: it takes over the default `m s` (select the pair) and removes `m w`.

```scheme
(load-plugin! "core:helix-surround")
```
