# Edit Operations: Acting on Selections

## The select-then-act model

In HUME, edit operations never act on a bare cursor position. They act on a
selection set: an ordered list of selections with one marked primary.
Selections are always **inclusive**. A single-position selection covers one
character, never a zero-width point. Each selection is either:

- **Single-character** (`anchor == head`): the cursor sits on one character.
- **Multi-character** (`anchor != head`): a contiguous region of selected
  text.

An operation like "insert character `x`" means:

- For a **single-character selection**: insert `x` before the cursor's
  character. The cursor stays on that character, which now sits one place to
  the right.
- For a **multi-character selection**: replace the selected region with `x`,
  and land the cursor after it.

Single-cursor editing, editing a selection, and multicursor editing all run
through the same loop.

## Multi-selection edit ordering

A selection set can hold several selections at once (multicursor). When an
edit touches several positions, the order of application matters: inserting
a character at offset 0 shifts every position to its right, so applying edits
one by one would corrupt the offsets that follow.

HUME avoids this with an edit builder. Each operation names a position or
range of the **original buffer**, and the builder handles the translation.
Operations are recorded against the old text and applied in position order,
so the order a command records them in does not change the result.

Because every selection records its own operations, ranges can overlap even
though the selections themselves do not. Take two cursors in the same word,
each pressing delete-word-backward in insert mode: each deletes back to the
word's start, so one range contains the other. The builder merges
overlapping deletions into their union, so the shared characters are removed
once. A position strictly inside a deleted range ends up at the point where
the deletion happened. The selections an edit leaves behind are resolved
once, against the new text, and selections that end up sharing a character
merge. See [Changesets](changesets.md).

## Primary vs secondary selections

All selections are **equal for editing**: insert, delete, and motions apply
to every selection in the set at once. The primary is the focused one. It is
distinguished in four situations:

1. **Status bar**: shows the primary's line and column. The bar cannot show
   every cursor, so one has to be canonical.

2. **Viewport scrolling**: the editor scrolls to keep the primary visible.
   Other cursors may be off-screen.

3. **Single-selection commands**: "keep primary" drops every selection except
   the primary (its extent is preserved); "remove primary" removes it and
   promotes the next one. Both act on one selection, identified by the
   primary.

4. **Registers**: yanking with N selections stores N entries, one per
   selection in document order. Pasting with N cursors gives each cursor its
   own entry. With a different number of cursors, every cursor gets all the
   entries joined. See [Registers, the Kill Ring, and
   Smart-p](kill-ring-and-smart-p.md#several-entries-a-different-number-of-cursors).

Paste also reads the selection state itself. Over a cursor, `p`/`P` inserts
after or before it. Over a selection of more than one character, paste
replaces the selected text. The selection already says whether the user
selected something on purpose, so no separate "replace" command is needed.
See [Repeat vs. swap](kill-ring-and-smart-p.md#repeat-vs-swap) for the one
case where paste lands beside a selection instead.

**Why cycle the primary?** With several cursors and only a keyboard, cycling
forward and backward through primaries is how you focus a different cursor:
to scroll the viewport to it, read its position in the status bar, or remove
just that cursor. Cycling is the keyboard's version of clicking a cursor.

The primary is a pointer into the sorted set of selections. It is updated
whenever the set changes: merges, removals, and splits all adjust the pointer
so it keeps tracking the intended selection.

---

*See also: [Changesets](changesets.md) for how an edit is built from
operations; [Registers, the Kill Ring, and Smart-p](kill-ring-and-smart-p.md)
for registers and paste.*
