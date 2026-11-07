# Splits and Panes: One Buffer, Many Views

## Buffer, pane, and view

A **buffer** is the text and its editing history: what you would call "the
file" in casual conversation. A **pane** is a live view onto one buffer, a
rectangle of the screen with its own scroll position and its own selections.
Several panes can show the same buffer at once. Split a window and both
halves show the same file, but each can be scrolled to a different part of
it and have a different cursor.

The separation matters because "where the cursor is" and "what the text
says" are different kinds of fact. The text belongs to the buffer; the cursor
and the scroll offset belong to the pane you are looking through.

## What lives on a pane, not the buffer

Each pane owns:

- **Selections**: the pane's own cursors. Two panes on the same buffer have
  independent selections, and moving the cursor in one never moves it in the
  other.
- **Viewport**: scroll position and pane size. A pane also remembers where
  it last scrolled to in *every* buffer it has shown, so switching back to a
  buffer restores the old scroll position instead of jumping to the top.
- **Wrap-mode override**: whether and how long lines soft-wrap normally comes
  from the buffer's own setting, or the editor-wide default. A pane can pin
  its own wrap style instead, so one pane can show a file wrapped while
  another, split from it, shows it unwrapped. Like the scroll position, the
  override is remembered per buffer: pinning one buffer's wrap style does not
  change how the pane shows another, and switching back restores it.

The buffer keeps the text, the undo history and its own settings (its
default wrap style among them), and nothing about how any pane is looking at
it.

## Keeping panes in sync

When one pane edits a buffer that another pane also shows, the other pane's
selections would go stale: a cursor at character 40 means something else
once ten characters are inserted at position 10.

So every edit propagates. Every other pane that has ever shown the buffer
has its selections carried through the same edit, and a cursor after the
insertion point moves along with its text. That includes a pane that has
since switched to another file: its remembered place in this one follows the
edits too.

Selections are not the only positions that follow. Jump lists, positions a
script is tracking, prompt snapshots, diagnostics and the other stored
positions are carried through the same change at the same moment. See
[Changesets: Describing Edits as Data](changesets.md) for how an edit maps an
old position to its new one.

## Layout: a tree of splits

Panes are arranged in a tree of rows and columns, the model most terminal
multiplexers and modal editors use. Splitting a pane the way its row already
runs adds a pane to that row; splitting it the other way replaces it with a
small column (or row) of two. So a row is always one node of the tree,
however many times it was split, and the dividers between its panes are the
boundaries between its members.

One pane is always **focused**: it receives the keystrokes. Unfocused panes
are dimmed, so it is obvious at a glance where your typing will land.

Where two panes meet, a thin seam is drawn between them (the
`pane-dividers` setting turns it off; dimming works either way). Where seams
meet, a junction glyph (a T or a cross) joins them cleanly. The seam
segments beside the focused pane are accent-coloured, a second focus cue
next to dimming. Seams exist only in the rendered frame, not in the buffer
or in the layout tree.

## Panes, bands, and overlays

Not everything on screen is a pane. Three kinds of surface share the frame,
and each behaves differently when the window is resized or something
temporary needs to appear:

- **Panes** are the leaves of the split tree, each with its own buffer,
  cursor and scroll position. Splitting subdivides them, closing one hands
  its space to its neighbours, and every pane can take focus.
- **Bands** are full-width strips reserved above or below the pane area: the
  status line, an optional tab bar, a scrolling list at the bottom of the
  screen. A band has no buffer and no cursor, and is not part of the split
  tree. Reserving one shrinks every pane as a terminal resize would, so a
  band never covers content; it makes room for itself.
- **Overlays** float on top of a pane and reserve no space: a completion
  list, a selection menu, a hover popup. What is underneath is covered, not
  moved, which is why an overlay suits something transient you dismiss
  rather than something you work inside.

## Splitting

Splitting the focused pane creates a new pane next to it, vertically
(side by side) or horizontally (stacked), showing the same buffer. The new
pane starts from the source pane's view: its selections, scroll position,
jump history and any wrap-mode override. From there the two diverge as you
work in each.

A row nobody has resized stays even: every split there resizes its panes
back to equal sizes, so splitting a third time gives three equal thirds
rather than halving the pane you split. A group of stacked panes counts as
one column (and a group of side-by-side panes as one row) when measured
against its neighbours, the way most terminal multiplexers balance a mixed
grid. Closing a pane works the same way in reverse: its space is shared
equally among the panes that remain.

## Resizing

Growing a pane moves one divider: the one after it in its row, or the one
before it when it is the last. Only the two panes beside that divider change
size, which is also what dragging a divider with the mouse does. A divider
keeps its place as a fraction of its row, so resizing the terminal scales the
whole row rather than squeezing only its last pane.

Once a row has a divider placed by hand, it stops being evened out. A split
there halves the pane you split, and closing a pane gives its space to the
neighbour beside it, so a split followed by a close puts the row back as it
was. Equalizing makes every row even again.

---

*See also: [The Rendering Pipeline](rendering-pipeline.md) for how a pane's
content becomes styled terminal cells (splitting runs that pipeline once per
pane, and its Stage 4 composes the status line, tab bar and bottom band
around the panes); [Changesets: Describing Edits as Data](changesets.md) for
how a non-editing pane's selections stay meaningful after another pane
edits the shared buffer.*
