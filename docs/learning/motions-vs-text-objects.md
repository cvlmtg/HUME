# Motions vs Text Objects

## The conceptual split

Both motions and text objects take a cursor position and produce a selection.
The difference is in how the anchor of that selection is determined:

| Concept | Inner fn output | Anchor of resulting selection |
|---------|----------------|-------------------------------|
| Motion | new *head* position | determined by the motion mode (may come from old selection state) |
| Text object | absolute `(start, end)` range | `start` in move mode; in extend mode, *unions* with the existing selection |

A motion inner function only answers "where does the head go?". In move mode
(`h`, `l`, `j`, `k`), the anchor collapses to the new head, producing a
single-character selection. In extend mode, the anchor stays fixed and only
the head moves — growing the selection.

A text object bypasses the motion mode in **move mode only**: it returns a
complete range and the framework creates a fresh selection from start to end,
discarding the previous anchor. In extend mode the matched range is *united*
with the current selection instead — the new start is the earlier of the two
starts, the new end the later of the two ends, and the existing direction is
preserved. The extend variant also does an outward-growth retry: when the first
match is a subset of what is already selected, it resumes the search from one
past the current end, so repeated extend-mode bracket and quote objects climb to
the enclosing pair rather than re-reporting the inner one.

Word motions (`w`/`b`/`W`/`B`) sit in between: navigational like motions but
returning a full word range. They use a third framework, word select,
described in [Word Motions](word-motions.md).

Structural navigation — `goto-next-function`/`goto-prev-function` and the
matching pairs for the other tree-sitter object kinds (class, comment, test,
argument), plus the paragraph motions (`{`/`}`) — is a fourth pattern,
combining pieces of the other three. Like a text object, each step returns a
whole object span rather than just a coordinate. Like a motion, it's a
repeatable, count-driven search that can no-op: pressing it past the last
object in the buffer leaves the selection where it already was rather than
producing a new one. Its extend mode borrows the text object's growth rule
rather than the plain motion one: instead of pinning the anchor and moving
only the head, each further press *unites* the newly found object with
whatever is already selected — so growing across several objects in a row,
or over one nested inside the one just selected, never loses ground already
covered. The paragraph motions reach this same pattern through a lexical
scan for blank-line boundaries rather than a tree-sitter query — the pattern
doesn't care how the object was found, only that a whole span comes back.

This leads to four framework entry points — one per pattern: motion, text
object, word select, and structural navigation.

## The inner function pattern

Both frameworks follow the same design: the inner function is *pure and
ignorant of multi-cursor*. It receives one position and returns one result.
The framework function handles iterating over all selections and merging any
that converge to the same range.

A motion inner function answers a coordinate question — "where does the cursor
go?" — and returns a position. A text object inner function returns a range, or
nothing if no match exists at the current position. On "no match", the existing
selection is preserved — pressing inner-bracket when not inside any brackets is
a no-op.

## Auto-merge after every motion or text object

After every motion or text object, selections that have converged to the same
range are automatically merged into one. This is essential for multicursor
correctness: if two cursors are both inside the same bracket pair and you press
inner-bracket, you want one combined selection, not two identical overlapping
ones.
