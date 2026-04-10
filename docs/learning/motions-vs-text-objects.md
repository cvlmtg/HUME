# Motions vs Text Objects

## The conceptual split

Both motions and text objects take a cursor position and produce a selection.
The difference is in how the anchor of that selection is determined:

| Concept | Inner fn output | Anchor of resulting selection |
|---------|----------------|-------------------------------|
| Motion | new *head* position | determined by `MotionMode` (may come from old selection state) |
| Text object | absolute `(start, end)` range | always `start` — independent of previous selection |

A motion inner function only answers "where does the head go?". With
`MotionMode::Move` (`h`, `l`, `j`, `k`), the anchor collapses to the new head,
producing a single-character selection. With `MotionMode::Extend`, the anchor
stays fixed and only the head moves — growing the selection.

A text object bypasses `MotionMode` entirely. It returns a complete range and
the framework always creates `Selection::new(start, end)` — the previous
anchor is discarded.

Word motions (`w`/`b`/`W`/`B`) sit in between: navigational like motions but
returning a full word range. They use a third framework, `apply_word_select`,
described in [Word Motions](word-motions.md).

This leads to three framework functions: `apply_motion`, `apply_text_object`,
and `apply_word_select`.

## The inner function pattern

Both frameworks follow the same design: the inner function is *pure and
ignorant of multi-cursor*. It receives one position and returns one result.
The framework function handles iterating over all selections and merging any
that converge to the same range.

```rust
// Motion inner function: position → position
fn move_right(buf: &Buffer, head: usize) -> usize { ... }

// Text object inner function: position → Option<(start, end)>
fn inner_word_impl(buf: &Buffer, pos: usize) -> Option<(usize, usize)> { ... }
```

Returning `Option` from a text object inner function means "no match at this
position". On `None`, the existing selection is preserved — `mi(` when not
inside parens is a no-op.

## `map_and_merge`

Both `apply_motion` and `apply_text_object` use `map_and_merge` on the
`SelectionSet`. After mapping each selection through the inner function, any
selections that have converged to the same range are automatically merged into
one. This is essential for multicursor correctness: if two cursors are both
inside the same bracket pair and you press `mi(`, you don't want two identical
overlapping selections — you want one.
