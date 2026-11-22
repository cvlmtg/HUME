# core:markdown — Lists

The list query (`md-lists/query`) runs over the lines Enter's cursors are on, the lines
`o` and `O` open from, the lines `G X`'s selections touch, and the lines an edit changed.
Its marker patterns are rooted at `list_item`, so a task box comes in a match with the
marker before it. An ordered marker also comes in a match rooted at its `list`, so every
ordered marker of a list on those lines comes back for renumbering, however far the list
runs.

## Prefixes

`md-lists/item` reads a line's prefix:

- **Item line:** a list marker starts on it. The prefix runs past the last marker, a task
  box right after it, and the blanks that follow. The box comes from
  `md-preview/task-box`, which the preview draws with too.
- **Quote in an item:** a `>` follows the last marker, as in `- > foo`. The prefix runs
  past the last such `>` and its blanks, and the line has no marker of its own.
- **Quote line:** no marker, but a `block_quote_marker` or `block_continuation` holds a
  `>`. The prefix runs past the last `>` and its blanks.
- **Otherwise** `#f`, an item's continuation line included, whose `block_continuation` is
  blanks only.

`md-lists/next-prefix` copies the prefix, with each marker before the last turned into
blanks (`1. - foo` gives `   - `), the last marker keeping its bullet or taking the new
number, and a task box becoming `[ ]`. Enter continues a line only for a cursor past its
prefix.

## Ending a list

When every cursor is on an empty item or quote line, Enter breaks no line, and
`md-lists/ended-edit` deletes the end of each prefix:

- **Item line:** what follows the last `>` before the first marker and the blank after it,
  or the whole prefix with no such `>`. So `- [ ] ` and `- > ` give an empty line, and
  `> 1. ` gives `> `.
- **Quote line:** the last `>` and what follows it. So `> > ` gives `> `.

A deletion, unlike a replacement, leaves the cursor at the end of what is kept.

## Renumbering

`md-lists/renumber` takes events, each a marker and a kind:

| Kind | From | Meaning |
|---|---|---|
| `'after`, `'before` | Enter and `o`; `O` | A new item after or before the marker |
| `'anchor` | an edit | The marker's number was edited: it keeps it, and the items after count on from it |
| `'resume` | an edit | The items after the marker count on from it, past removed items |
| `'start n` | an edit | The marker takes `n`, the number of the removed first item before it |

`md-lists/numbered-lists` reads each list once, with one `buffer-lines` call over its
markers' lines, and the walk reads these records, not the buffer. Only ordered markers
count. The walk starts at a list's first event: earlier items keep their numbers, and each
later slot, new items included and removed ones skipped, takes the next number. So a new
item after `n.` is `n+1`, before `n.` is `n`, and a removed `n.` hands `n` to the item
after it. A wider number does not move the text after the marker.

A list whose items share one number, as in `1.`, `1.`, `1.`, keeps it, since Markdown
counts on from the first item anyway. That holds when at least two of its items have no
`'anchor` and share one number: the walk makes no edit, and each new item takes that
number.

The result is the edits and, per event, its new item's number, `#f` for a bullet or a line
with no marker.

## Renumbering after an edit

The `on-text-changed` handler runs right after the keystroke, so `syntax-captures` parses
the edited text on the spot. It skips a read-only buffer and an event whose hunks are
`#f`, and edits through the first pane `buffer-panes` returns.

**Which hunks.** `md-lists/edited-hunks` keeps a hunk when, at some column, its old and
new lines hold different ordered-marker numbers, so typing in an item's text drops the
hunk before the tree is read. Both sides go through `md-lists/line-markers`, the prefix
reader the table commands use, so a line it misreads counts the same on both. Edits on
adjacent lines arrive as one hunk, as Enter at two adjacent items gives, so a kept hunk is
split by `diff-lines` into the lines it changed, and the item between two new ones counts
on instead of anchoring. A pure insertion or deletion stays whole, since `diff-lines`
reads an empty text as one empty line.

**Which lists.** `md-lists/numbered-query`, the list pattern alone, runs over each kept
hunk's new lines, reaching back to the last non-blank line before them and on to the first
one after, so a list that now starts after a blanked item comes back. Per list, a hunk
that changed one of its marker columns gives (`md-lists/list-events`):

- an `'anchor` for each of the list's markers on the hunk's new lines: an edited number, a
  pasted item, an item Enter, `o` or `O` added, an item outdented into the list;
- with none, a `'resume` on the list's last item before the hunk: deleted items, or an
  item indented into a nested list;
- with no item before the hunk either, a `'start` on its first item after, carrying the
  first number the hunk's old lines held at that column with the list's delimiter. So
  deleting `1.` from `1.`, `2.`, `3.` leaves `1.`, `2.`, and deleting `3. a` leaves a `1)`
  list after it alone, since a new delimiter starts a new list.

A later anchor restarts the counting, so a pasted run of items keeps its own numbers.

**Idempotence.** The pass leaves every list it touched counting on, so a second pass makes
no edit. It stores the generation its edit made (`md-lists/*renumbered*`) and skips the
next event when the buffer is still at that generation. An undo is an edit like any other:
it renumbers nothing in a list numbered in order or with one number, but can renumber one
numbered neither way, as `3.`, `7.`, `3.`.

**Undo.** Typed in Insert mode, the renumbering joins the session's undo group. A
Normal-mode edit such as `r1`, a `.` or a macro has made its undo step before the pass
runs, so the renumbering is a second one.

## Order of edits

A command reads the text and plans every edit before making one, since its first edit
changes the tree: after an Enter break, the new line between two quote lines has no `>`,
which ends the quote and splits the list, and `o` and `O` leave a blank line. It writes
only its new items' prefixes, numbered by one `md-lists/renumber` walk, so several cursors
in one list number together. The items after them are renumbered by the pass after the
edit, which runs while the command's Insert session is open and so joins its undo group.

- **Enter:** `insert-key!` breaks every line as Enter does, with smart indent, auto-pairs
  and the autoindent record. One `edit-text!` then replaces the blanks before each
  continued cursor with its prefix, matched by index, since selections keep their count
  and order through the break. Ending a list is one `edit-text!` of the ended prefixes.
- **`o` and `O`** start in Normal mode, where an edit before `open-line-below` or
  `open-line-above` would be an undo step of its own. So the open runs first, and one
  `edit-text!` writes the prefixes.

## Task boxes

`markdown/toggle-task` reads every line a selection touches as `md-lists/item` does. A
line's box is its item's `'task`, the box right after its last marker, so `1. - [ ] a`
toggles the inner item's box. The edit replaces only the character between the brackets,
so a cursor or selection end on the box stays put.
