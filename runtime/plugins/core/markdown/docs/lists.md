# core:markdown — Lists

Enter, `o`, `O` and `G X` in a Markdown buffer go through the `"markdown/lists"` keymap.
The `on-buffer-open` hook attaches it, and `on-buffer-close` detaches it from a buffer that
stays open with another language. `markdown/enter` handles Enter, `markdown/open-below` and
`markdown/open-above` handle `o` and `O`, and `markdown/toggle-task` handles `G X`. With
`'continue-lists` off, only `G X` is bound and no `on-text-changed` handler is registered.

The list query (`md-lists/query`) runs over the lines Enter's cursors are on, the lines `o`
and `O` open from, every line a selection touches for `G X`, and the lines an edit changed
for [renumbering after it](#renumbering-after-an-edit). Its marker patterns are rooted at
`list_item`, as the block query's bullet pattern is, and a task box comes in a
match with the marker before it, as `@task-checked` or `@task-unchecked`. An ordered marker
also comes in a match with the list holding it, rooted at `list`, so every ordered marker of
a list on those lines comes back for renumbering, however far the list runs. A line in a
fenced block that is not Markdown has no marker, so Enter, `o` and `O` act there as they
always do.

Enter skips the query when no cursor's line starts, past blanks, with a digit, `-`, `*`, `+`
or `>` (`md-lists/may-start-prefix?`): only such a line can hold a prefix, so the query
would find no item, and Enter breaks the line at once.

## Prefixes

`md-lists/item` reads one line:

- **Item line:** a list marker starts on it. Its prefix runs from the start of the line past
  the last marker, past a task box right after it, and past the blanks that follow. The
  box comes from `md-preview/task-box` (`queries.scm`), which the preview also draws boxes
  with and which reads a box the grammar misses from the line
  ([Node shapes](elements.md#node-shapes)).
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

`md-lists/renumber` takes events, each a marker and a kind. Enter, `o` and `O` give one per
new item, to number it:

- `'after`: a new item after the line's last marker (Enter, `o`);
- `'before`: a new item before it (`O`).

An edit gives three more (see [Renumbering after an edit](#renumbering-after-an-edit)):

- `'anchor`: the marker's number was edited, so it keeps that number and the items after it
  count on from it;
- `'resume`: the items after the marker count on from it, past the removed items between;
- `'start`, carrying a number `n`: the marker takes `n`, the number of the removed first
  item before it.

Only ordered markers count. Each list that holds an event is walked in order. The walk starts
at its first event: an item before it keeps its number, and each later slot, new items
included and removed ones skipped, takes the next number. So the first new item after `n.`
is `n+1`, a new item before `n.` is `n`, and a removed `n.` hands `n` to the item after
it. Several cursors in one list number together. A wider number does not move the text after
the marker.

A list whose items all carry one number, as in `1.`, `1.`, `1.`, keeps it, since Markdown
counts on from the first item whatever the others say. A list is read that way when at least
two of its items have no `'anchor` event and those share one number. The walk then makes no
edit in it, and each new item takes that number.

The result is the renumbering edits and, per event, its new item's number. A bullet gets
`#f`, and so does an event with no marker: a quote line's, or a plain line's. The commands
use the numbers and the edit pass the edits.

## Renumbering after an edit

When an edit changes the numbers of a list's items, the items after it count on from the
last item it left: changing `3.` to `1.` turns the `4.` after it into `2.`, and deleting
`2.` turns the `3.` after it into `2.`. The items before it keep theirs. The pass runs from
the `on-text-changed` handler, right after the keystroke, and `syntax-captures` parses the
edited text on the spot. While typing, the renumbering joins the Insert session's undo
group. A Normal-mode edit such as `r1`, and a `.` or a macro, which replay a whole Enter
or `o` before the pass runs, have already made their undo step, so the renumbering is a
second one. The handler skips a read-only buffer
and an event whose hunks are `#f`.

`md-lists/line-markers` reads the ordered markers at the start of a line, past blanks,
`>` and bullets, each with its column, number and delimiter. `md-lists/edited-hunks` keeps a hunk when, at some
column, the numbers its old lines hold there differ from those its new lines hold. Typing
in an item's text, deleting a word, or rewriting a line around its marker leaves them equal,
so that hunk is dropped before the tree is read. Both sides go through the same reader, so a
line it misreads counts the same on both. Edits on touching lines arrive as one hunk, as
Enter at two adjacent items gives, so a kept hunk is split by `diff-lines` into the lines it
changed: the item between the two new ones then counts on instead of anchoring. A hunk with
no lines on one side, a pure insertion or deletion, is one change already and is kept whole,
since `diff-lines` reads an empty text as one empty line.

The list query runs over each kept hunk's new lines, reaching back to the last non-blank
line before them and on through the first one after them, so a list that now starts after
a blanked item or a blank line comes back with them. For each list it returns, a hunk that
changed one of the list's marker columns gives `md-lists/list-events`:

- an `'anchor` for each of the list's markers on the hunk's new lines: an edited number, a
  pasted item, an item Enter, `o` or `O` added, an item outdented into the list;
- with none there, a `'resume` on the list's last item before the hunk: deleted items, or an
  item indented into a nested list;
- with no item before the hunk either, a `'start` on its first item after, carrying `n`,
  the first number the hunk's old lines held at that column with the list's delimiter, so
  deleting `1.` from `1.`, `2.`, `3.` leaves `1.`, `2.`. A marker with the other delimiter
  belonged to another list, since a new delimiter starts a new list, so deleting `3. a`
  leaves the `1)` list after it alone.

A later anchor starts the counting again, so a pasted run of items keeps its own numbers.
The pass's own edit leaves every list it touched counting on, so another pass over it would
make no edit. The pass keeps the generation its edit made (`md-lists/*renumbered*`) and
skips the next event when the buffer is still at that generation. Any other edit since
moves the generation, and the pass reads the whole change.

An undo is an edit like any other. For a list numbered in order, or with one number on every
item, it makes no renumbering. A list numbered neither way, as `3.`, `7.`, `3.`, can be
renumbered by an undo.

The edit goes through the first pane `buffer-panes` returns, the focused one when it shows
the buffer.

## Order of edits

Edits made by an Insert-mode binding join the session's undo group, so one `u` undoes the
whole key. Each command reads the text and plans before it edits, since the edit it makes
first changes the tree:

- After an Enter break, the new line between two quote lines has no `>`, so it ends the
  quote and splits the list.
- `o` and `O` leave a blank line.

A command writes only its new items' prefixes, numbered by one `md-lists/renumber` walk over
its events, so several cursors in one list number together. The items after them, and the
items after an ended list, are renumbered by the [pass after the edit](#renumbering-after-an-edit),
which runs at the settle after the key, while the Insert session the command opened is still
open, so it joins the same undo group.

**Enter:** `insert-key!` breaks every line as Enter does, with smart indent, auto-pairs and
the autoindent record. A last `edit-text!` replaces the blanks before each continued cursor
with its prefix. Selections keep their count and order through the break, so the prefixes
are matched to them by index. Ending a list is one `edit-text!` of the ended prefixes.

**`o` and `O`** start in Normal mode, and an edit made before `open-line-below` or
`open-line-above` would be an undo step of its own. So the open runs first, and one
`edit-text!` writes the prefixes.

## Task boxes

`markdown/toggle-task` reads every line a selection touches, from the line of its first
character to the line of its last, as `md-lists/item` reads it. A line's box is its item's
`'task`, the box right after its last marker, so `1. - [ ] a` toggles the inner item's box.
The first box on the primary selection's lines decides, or the first box of all when those
lines hold none: when it is checked, `[x]` or `[X]`, every box becomes `[ ]`, and otherwise
every box becomes `[x]`. Only a box whose state changes gets an edit, and a line with no box
gets none. The edit replaces only the character between the brackets, so a cursor or
selection end on the box stays where it was.
