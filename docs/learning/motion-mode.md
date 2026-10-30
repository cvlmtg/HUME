# Move vs Extend: Separating Position from Anchor Semantics

In most text editors, "move the cursor" and "extend the selection" are bound
to separate keys: arrow keys move, Shift-arrow extends. In HUME, the two
behaviours are the same command with a different mode parameter. This is why
`h` can both move the cursor (normal use) and grow or shrink a selection (in
extend mode) without a separate `"extend-left"` command. Only the head moves
while the anchor stays fixed, so moving away from the anchor grows the
selection and moving back toward it shrinks it. Grow and shrink are one
mechanism, not two features.

## A concrete walkthrough

Buffer: `"hello world\n"`, cursor on `'h'` (position 0). The selection is a
single-character selection on `'h'`: anchor = 0, head = 0.

Pressing `l` invokes the motion framework in move mode, with a count of 1,
and a one-step move-right search.

**Step 1: the search.** Move-right computes the next grapheme boundary from
position 0, returning 1. It knows nothing about anchors or multi-cursor; it
is a coordinate calculation.

**Step 2: apply move mode.** The framework collapses anchor and head to the
new position:

```text
move → anchor = 1, head = 1   (single-char selection on 'e')
```

Now suppose the cursor is at position 2 (on `'l'`) and the user presses `l` in
extend mode:

**Step 2: apply extend mode.** The framework keeps the old anchor and moves
only the head:

```text
extend → anchor = 2 (unchanged), head = 3
```

The selection grew from `'l'` to cover both `'l'` characters while the anchor
stayed put. Pressing `h` from here would move the head back toward the
anchor, shrinking the selection: the same computation, walked in the other
direction.

## The two modes

| Mode | Anchor | Head | Typical use |
|------|--------|------|-------------|
| move   | the new head   | the new head | Plain cursor move: `h`, `j`, `k`, `l` |
| extend | the old anchor | the new head | Grow or shrink selection: sticky extend mode (toggled by `e`), one-shot Ctrl-letter on kitty-capable terminals |

Move mode always produces a collapsed single-character selection (anchor ==
head). Extend mode keeps the existing anchor, only moving the head.

## Why separate the search from the mode

The search is a pure coordinate calculation. It knows nothing about anchors
or multi-cursor, though it is given the whole selection to look at. The
move/extend mode is a concern of the dispatch layer, not of the motion
itself. This means:

- Adding a new motion (e.g. "go to line end") requires one search; the move
  and extend variants follow from the mode.
- Testing the motion means asserting on the returned position.
- The same move-right search powers both `l` in move mode and `l` in extend
  mode, with no separate command.

The framework branches on the mode when constructing the resulting
selection:

```text
move   → anchor = new head, head = new head    (collapsed single-char)
extend → anchor = old anchor, head = new head  (anchor stays, head moves)
```

Every position in these rules is the start of a character cluster, and a
selection end covers the whole cluster that starts there. A head moved onto
an accented letter therefore covers the letter and its accent together.

A few anchor-manipulation commands sit beside this framework without being a
new mode. Overshooting a target is corrected by walking the head back: the
same motion, reversed, shrinks the selection. `Ctrl-e` flips which end of
the selection is the head and which is the anchor, for when you would rather
grow or shrink from the *other* end. `Ctrl-;` (on kitty-capable terminals)
collapses the selection onto its anchor, discarding the head. All of these
reuse the positions the existing selection already carries.

---

*See also: [Motions vs Text Objects](motions-vs-text-objects.md) for the
other ways a selection is built; [The Command/Keymap/Dispatch
Architecture](command-keymap-dispatch.md) for how dispatch passes the mode.*
