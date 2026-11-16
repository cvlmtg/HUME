# Conceals and the Markdown Preview

A Markdown file reads better as a bold `Title` than as `## Title`, and a
link reads better as its text than as `[text](https://…)`. The Markdown
preview draws a buffer that way while you edit it. Underneath it is one
general mechanism any plugin can use: the **conceal**.

## What a conceal is

A conceal draws part of one line as something other than its text. It comes
in two forms:

- **Hide**: a run of characters is left out of the drawing, and optional
  replacement text is drawn in its place. The `[`, `](` and URL around a
  link's text are hidden with nothing in their place; a heading's `## ` is
  replaced by a small marker showing its level; a `---` rule is replaced by a
  line of `─`.
- **Insert**: text is drawn before a character, hiding nothing. A table's
  border bars are inserts.

The buffer never changes. Saving writes the source, undo sees no edit, and
every command acts on the real characters. A conceal is a decoration, like an
inlay hint, except that it can remove what is drawn instead of only adding
to it.

A conceal always covers whole characters, never half of a combined emoji
(see [Unicode Position Model](unicode-position-model.md)), and it stays on
its own line: it can neither hide a line break nor draw one. Its replacement
can carry its own colour, drawn over the highlight of the text it hides.

Replacement text stands for the characters it hides, so the cursor can stand
on it; it sits on the first character hidden. Inserted text stands for
nothing, so the cursor steps over it as it steps over an inlay hint.

## Revealing: edit what you see

Editing text you cannot see is how mistakes happen: deleting a word could
take the hidden `**` around it with it. So every line a selection touches
shows its source. Move onto a heading and its `## ` comes back; move away and
it hides again.

A conceal can also name further lines that reveal with its own. A table or a
fenced code block uses this to show its source as a whole while any of its
lines holds a selection, since half a table drawn and half raw reads as
neither.

A popup has no selection, so nothing in it reveals: a Markdown hover or
completion documentation box is always drawn as a preview.

## One layout for drawing, one for moving

A conceal changes how wide a line is, and so where it wraps and how many
display lines it takes (see [Display Lines and Soft
Wrap](display-lines-and-soft-wrap.md)). Every walk that turns positions into
rows must agree on that width, so the layout counts conceals the same way the
renderer draws them.

Revealing makes one more thing necessary. When `j` moves the cursor onto a
concealed line, that line reveals, and its source is often wider than what
was drawn: `[docs](https://example.com)` shows as `docs`. Aiming for "the
same column" in the drawn line would land the cursor somewhere unrelated once
the line opens up. So a pane with conceals keeps two layouts:

- the **drawn** layout, what is on screen, used for painting, mouse clicks
  and scrolling;
- the **source** layout, every line revealed, used by `j` and `k` to pick the
  target column.

A buffer with no conceals has one layout, and moving the cursor never
changes it.

## Several lists, one line

A plugin hands over its conceals as a named list, and several lists can
conceal the same buffer. Within one list, two conceals that overlap are an
error, unless the plugin asks for the later one to be dropped. Between
lists, the editor takes them in order of their names, and a conceal that
overlaps one already kept on its line is dropped whole. Dropping it whole
rather than trimming it keeps every conceal either drawn as its plugin meant
or not drawn at all.

## How the preview is built

The Markdown preview is a plugin, and it uses only what any plugin can:

1. It asks the syntax tree for the Markdown nodes on the visible lines, with
   a query of its own (see [Tree-sitter](tree-sitter-pipeline.md#plugins-reading-the-tree)).
   The visible lines are widened to whole nodes, so a table that starts
   above the screen is still drawn as a table.
2. It turns those nodes into conceals: heading markers hidden, emphasis
   markers hidden, links reduced to their text, code fences hidden, table
   bars inserted.
3. It colours what stays: bold headings, and background bands across
   headings and code blocks.
4. It does it again each time a fresh parse lands or the view scrolls.

A popup asks the same plugin to format its text, and draws the result
through the same layout and style stages a pane line goes through (see [The
Rendering Pipeline](rendering-pipeline.md)), so a hover renders as the
buffer would, except that the popup wraps the drawn text to its own width.

What the design gives up: replacement text cannot wrap or break, so a rule or
a table wider than the pane is cut at its edge.

---

*See also: [The Rendering Pipeline](rendering-pipeline.md) for where conceals
enter the frame; [Display Lines and Soft Wrap](display-lines-and-soft-wrap.md)
for the other decorations that change a line's layout.*
