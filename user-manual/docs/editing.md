# Editing

Say what you mean, then say what to do with it. Because the selection always comes first, you can see exactly what an edit will touch before it happens — and the same handful of action keys work on a character, a word, a block, or forty places at once.

## Inserting text

| Key | Where insertion begins |
|-----|----------------------|
| `i` | Before the selection |
| `a` | After the selection |
| `I` | First non-blank character on line |
| `A` | End of line |
| `o` | New line below |
| `O` | New line above |

Press `Esc` or `Ctrl+c` to return to Normal mode. `Ctrl+w` deletes the word before the cursor while you type.

## Deleting and changing text

| Key | Effect |
|-----|--------|
| `d` | Delete selection (pushed onto the kill ring) |
| `c` | Delete selection content and enter Insert mode (one undo group). A trailing newline is kept — `c` on a line rewrites its content without removing the line itself. |

Use `x` to select the current line first if you want a line-wise delete (`x` then `d`).

See [Copy & Paste](copy-and-paste.md) for what the kill ring is and how to paste from it.

`c` keeps the selection on the text you changed: leaving Insert mode selects your replacement instead of leaving a plain cursor, so you can immediately act on it again — delete it, surround it, search for it. Pressing `Esc` without typing anything leaves the cursor where the change began. Disable this with the `select-changed-text` option (see [Configuration](configuration.md)).

Whichever way you entered Insert mode, `m i i` recovers what you last typed after `Esc` — see [Text objects](selections.md#text-objects).

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

## Lines and alignment

| Key | Effect |
|-----|--------|
| `J` | Join the selected lines into one |
| `&` | Align selections into a column |
| `>` | Indent lines touched by a selection |
| `<` | Unindent lines touched by a selection |

`J` replaces each line break with a single space and drops the next line's indentation, so joining wrapped code or prose doesn't leave a gap in the middle. When the next line is empty or only whitespace, the lines are joined with no space at all. Joining several lines at once leaves one cursor on each inserted space, ready to act on.

`&` lines up your selections using the primary selection's line as the starting point. Spaces are inserted at the left edge of each selection to reach that column, and if some other line needs more room, the column widens for everybody — which means the primary selection can shift right too. Multi-line selections are left alone. Where a selection sits too far right already, the run of spaces or tabs immediately to its left is squeezed down (never below one).

You can align to the left or the right depending on which end of the selection is the anchor; `Ctrl+e` swaps anchor and head. See [Selections](selections.md#flipping-and-collapsing-the-selection).

`>`/`<` shift every line touched by a selection by one indent level, using the buffer's `tab-width` and `tab-style` (see [Configuration](configuration.md)) — a prefix count shifts by that many levels at once (`3>`). A blank or whitespace-only line inside the selection is left alone, so it never picks up trailing whitespace. Each touched line's whole indent is re-rendered to the new width in the current `tab-style`, not just prepended to or trimmed from — so `>` after `<` (or vice versa) always gets you back to exactly where you started.

## Undo and redo

| Key | Effect |
|-----|--------|
| `u` | Undo (accepts a count — `5u` undoes five steps) |
| `U` / `Ctrl+r` | Redo |

Undo history is a tree rather than a straight line, so redoing after new edits follows the most recent branch. The history lives in memory only and starts fresh each time you open a file.

## Repeat

| Key | Effect |
|-----|--------|
| `.` | Repeat the last editing command |

Dot-repeat replays the most recent insert session or editing command — delete, change, paste and so on, but not `y`.

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

Recording is ignored in read-only buffers, and while a macro is already recording or replaying — so a macro can't record itself or nest.

## Numeric count

Prefix a command with digits to repeat it. The first digit must be `1`–`9`; `0` counts only once a count is already under way. So `12w` moves forward 12 words and `10j` moves down 10 lines. Counts above 10,000 are capped at 10,000.

| Example | Effect |
|---------|--------|
| `3w` | Move forward 3 words |
| `5j` | Move down 5 lines |
| `12w` | Move forward 12 words |
| `10j` | Move down 10 lines |

`0` on its own does nothing by default — `g h` goes to the start of the line. If you want vim's `0`, the `core:vim-keybind` plugin binds it (see [Core Plugins](core-plugins.md#core-vim-keybind)).

## Surround

HUME's surround commands select or wrap delimiter pairs using the `m` prefix — see [Selections](selections.md) for the full list of `m i`/`m a` text objects.

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

Note that this moves wrapping onto `m s`: it takes over the default `m s` (select the pair) and removes `m w`.

```scheme
(load-plugin "core:helix-surround")
```
