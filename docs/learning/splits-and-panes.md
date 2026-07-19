# Splits and Panes: One Buffer, Many Views

## Buffer, pane, and view

A **buffer** is the text and its editing history — the thing you'd call "the
file" in casual conversation. A **pane** is a live view onto exactly one
buffer: a rectangle of the screen with its own scroll position, its own
selections, and its own idea of whether long lines should wrap. Several panes
can point at the same buffer at once — split a window and both halves show
the same file, but each half can be scrolled to a different part of it and
have a different cursor.

This separation matters because "where the cursor is" and "what the text
says" are different kinds of fact. The text belongs to the buffer; the
cursor, scroll offset, and wrap setting belong to whichever pane you're
looking at.

## What lives on a pane, not the buffer

Each pane owns:

- **Selections** — the pane's own cursor(s). Two panes on the same buffer
  have completely independent selections; moving the cursor in one never
  moves it in the other.
- **Viewport** — scroll position and pane size. A pane also remembers where
  it last scrolled to in *every* buffer it has shown, so switching a pane
  back to a buffer it displayed earlier restores the old scroll position
  instead of resetting to the top.
- **Wrap mode** — whether long lines soft-wrap, and how. This is a view
  property, not a document one: one pane can show a file wrapped while
  another pane, split from it, shows the same file unwrapped.

None of this is buffer state. A buffer holds text and undo history and
nothing about how it's currently being looked at; a pane holds exactly the
"how it's being looked at" half.

## Keeping panes in sync

If two panes show the same buffer and one of them makes an edit, the other
pane's selections would go stale the instant the edit lands — a cursor at
character 40 means something different after ten characters were inserted at
position 10. Every edit therefore propagates: the editing pane applies its
edit normally, and every other pane currently viewing that buffer has its
selections carried forward through the same edit, so a cursor sitting after
the insertion point moves along with the text it was next to. See
[Changesets: Describing Edits as Data](changesets.md) for how an edit maps an
old position to its new one.

## Layout: a tree of splits

Panes are arranged in a recursive tree of horizontal and vertical splits, the
same model used by most terminal multiplexers and modal editors. Splitting a
pane doesn't just add a rectangle next to it — it replaces that pane in the
tree with a small subtree of two panes, so splitting one half of an existing
split nests correctly no matter how many times you've already split.

One pane is always **focused** — it's the one that receives keystrokes.
Focused and unfocused panes render differently: the unfocused ones are dimmed,
making it visually obvious at a glance which pane your typing will land in.

Where two panes meet, a thin seam is drawn between them, and where three or
more seams meet, a junction glyph — a T- or cross-shape — connects them
cleanly instead of leaving a gap or an overlap. This is purely a rendering
detail: the seam and its glyph exist only in the rendered frame, not in the
buffer or the layout tree's own data.

## Splitting

Splitting the focused pane creates a new pane next to it — vertically
(side-by-side, left and right) or horizontally (stacked, top and bottom) —
viewing the same buffer the original pane was showing. The new pane inherits
the source pane's current view state (scroll position, wrap mode) as its
starting point, then diverges independently from there as you scroll or
re-wrap it.

---

## See also

- [The Rendering Pipeline](rendering-pipeline.md) — how a single pane's
  content is turned into styled terminal cells; splitting just means running
  that pipeline once per pane and arranging the results.
- [Changesets: Describing Edits as Data](changesets.md) — the mechanism that
  keeps a non-editing pane's selections meaningful after another pane edits
  the shared buffer.
