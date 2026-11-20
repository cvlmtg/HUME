# core:markdown — Lists

Enter, `o` and `O` in a Markdown buffer go through the `"markdown/lists"` keymap. The
`on-buffer-open` hook attaches it, and `on-buffer-close` detaches it from a buffer that stays
open with another language. `markdown/enter` handles Enter, and `markdown/open-below` and
`markdown/open-above` handle `o` and `O`. With `'continue-lists` off, nothing is bound or
attached.

The list query (`md-lists/query`) runs over the lines the cursors are on. Its `@list` and
marker patterns are rooted at `list`, so every marker of a list on those lines comes back,
however far the list runs. A marker belongs to the smallest `@list` holding it. A line in a
fenced block that is not Markdown has no marker, so Enter, `o` and `O` act there as they
always do.

## Prefixes

`md-lists/item` reads one line:

- **Item line:** a list marker starts on it. Its prefix runs from the start of the line past
  the last marker, past a task box right after it, and past the blanks that follow.
- **Quote line:** no marker starts on it, but a `block_quote_marker` or
  `block_continuation` on it holds a `>`. Its prefix runs past the last of these and the
  blanks that follow.
- **Otherwise** it is `#f`: plain text, and an item's continuation line, whose
  `block_continuation` is blanks only.

The next line's prefix (`md-lists/next-prefix`) copies the prefix with these changes:

- each marker before the last becomes blanks of its width (`1. - foo` gives `   - `);
- the last marker keeps its bullet, or takes the new number with its own `.` or `)`;
- a task box becomes `[ ]`.

Enter continues a line only for a cursor past its prefix. A cursor inside the prefix, a
marker included, breaks the line as Enter does.

## Ending a list

When every cursor is on an item or quote line with nothing after its prefix, Enter breaks no
line. `md-lists/ended-edit` instead deletes the end of each prefix:

- **Item line:** what follows the last `>` before the first marker, and the blank after it.
  With no `>`, the whole prefix. So `- ` and `  - ` give an empty line, and `> 1. ` gives `> `.
- **Quote line:** the last `>` and what follows it. So `> ` gives an empty line, and `> > `
  gives `> `.

A deletion rather than a replacement leaves the cursor at the end of what is kept.

## Renumbering

`md-lists/renumber` takes events, each an item and a kind:

- `'after`: a new item after it (Enter, `o`);
- `'before`: a new item before it (`O`);
- `'remove`: the item is gone (an ended list).

Only ordered items count. Each list that holds an event is walked in order. The walk starts
at its first event: an item before it keeps its number, and each later slot, new items
included and removed ones skipped, takes the next number. So the first new item after `n.`
is `n+1`, a new item before `n.` is `n`, and a removed `n.` hands `n` to the item after
it. Several cursors in one list number together. A wider number does not move the text after
the marker.

The result is the renumbering edits and, per event, its new item's number. Bullet items get
`#f`.

## Order of edits

Edits made by an Insert-mode binding join the session's undo group, so one `u` undoes the
whole key. Each command reads the text and plans before it edits, since the edit it makes
first changes the tree:

- After an Enter break, the new line between two quote lines has no `>`, so it ends the
  quote and splits the list.
- `o` and `O` leave a blank line.

**Enter** applies the renumbering first, since Insert mode is already on. `insert-key!` then
breaks every line as Enter does, with smart indent, auto-pairs and the autoindent record.
A last `edit-text!` replaces the blanks before each continued cursor with its prefix.
Selections keep their count and order through the break, so the prefixes are matched to
them by index.

**`o` and `O`** start in Normal mode, and an edit made before `open-line-below` or
`open-line-above` would be an undo step of its own. So the open runs first. The planned
renumbering is then moved to the new text: each line shifts down by the lines opened at or
above it, and keeps its column. One `edit-text!` applies it with the prefixes.
