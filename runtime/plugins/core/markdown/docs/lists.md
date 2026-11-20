# core:markdown — Lists

Enter, `o`, `O` and `G X` in a Markdown buffer go through the `"markdown/lists"` keymap.
The `on-buffer-open` hook attaches it, and `on-buffer-close` detaches it from a buffer that
stays open with another language. `markdown/enter` handles Enter, `markdown/open-below` and
`markdown/open-above` handle `o` and `O`, and `markdown/toggle-task` handles `G X`. With
`'continue-lists` off, only `G X` is bound.

The list query (`md-lists/query`) runs over the lines Enter's cursors are on, the lines `o`
and `O` open from, and every line a selection touches for `G X`. Its marker patterns are
rooted at `list_item`, as the block query's bullet pattern is, and a task box comes in a
match with the marker before it, as `@task-checked` or `@task-unchecked`. An ordered marker
also comes in a match with the list holding it, rooted at `list`, so every ordered marker of
a list on those lines comes back for renumbering, however far the list runs. A line in a
fenced block that is not Markdown has no marker, so Enter, `o` and `O` act there as they
always do.

## Prefixes

`md-lists/item` reads one line:

- **Item line:** a list marker starts on it. Its prefix runs from the start of the line past
  the last marker, past a task box right after it, and past the blanks that follow. The
  grammar reads a box with nothing after it, as in `- [ ] `, as text, so `md-preview/task-box`
  (`queries.scm`, which the preview also draws boxes with) finds that box in the line instead.
- **Quote in an item:** a `>` follows the last marker, as in `- > foo`. The line is a quote
  line inside the item: its prefix runs past the last such `>` and the blanks that follow, and
  it has no last marker of its own.
- **Quote line:** no marker starts on it, but a `block_quote_marker` or
  `block_continuation` on it holds a `>`. Its prefix runs past the last `>` in these and the
  blanks that follow.
- **Otherwise** it is `#f`: plain text, and an item's continuation line, whose
  `block_continuation` is blanks only.

The next line's prefix (`md-lists/next-prefix`) copies the prefix with these changes:

- each marker before the last becomes blanks of its width (`1. - foo` gives `   - `, and
  `- > foo` gives `  > `);
- the last marker keeps its bullet, or takes the new number with its own `.` or `)`;
- a task box becomes `[ ]`.

Enter continues a line only for a cursor past its prefix. A cursor inside the prefix, a
marker included, breaks the line as Enter does.

## Ending a list

When every cursor is on an item or quote line with nothing after its prefix, Enter breaks no
line. `md-lists/ended-edit` instead deletes the end of each prefix:

- **Item line:** what follows the last `>` before the first marker, and the blank after it.
  With no `>`, the whole prefix. So `- `, `  - `, `- [ ] ` and `- > ` give an empty line, and
  `> 1. ` gives `> `.
- **Quote line:** the last `>` and what follows it. So `> ` gives an empty line, and `> > `
  gives `> `.

A deletion rather than a replacement leaves the cursor at the end of what is kept.

## Renumbering

`md-lists/renumber` takes events, each a marker and a kind:

- `'after`: a new item after the line's last marker (Enter, `o`);
- `'before`: a new item before it (`O`);
- `'remove`: the marker is gone, one event for each marker an ended list deletes, so ending
  `1. - ` renumbers the outer list too.

Only ordered markers count. Each list that holds an event is walked in order. The walk starts
at its first event: an item before it keeps its number, and each later slot, new items
included and removed ones skipped, takes the next number. So the first new item after `n.`
is `n+1`, a new item before `n.` is `n`, and a removed `n.` hands `n` to the item after
it. Several cursors in one list number together. A wider number does not move the text after
the marker.

The result is the renumbering edits and, per event, its new item's number. A bullet gets
`#f`, and so does an event with no marker: a quote line's, or a plain line's.

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

## Task boxes

`markdown/toggle-task` reads every line a selection touches, from the line of its first
character to the line of its last, as `md-lists/item` reads it. A line's box is its item's
`'task`, the box right after its last marker, so `1. - [ ] a` toggles the inner item's box.
The first box on the primary selection's lines decides, or the first box of all when those
lines hold none: when it is checked, `[x]` or `[X]`, every box becomes `[ ]`, and otherwise
every box becomes `[x]`. Only a box whose state changes gets an edit, and a line with no box
gets none. The edit replaces only the character between the brackets, so a cursor or
selection end on the box stays where it was.
