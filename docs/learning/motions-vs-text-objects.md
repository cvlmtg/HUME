# Motions vs Text Objects

## The conceptual split

Both motions and text objects look at the current selection and produce a
new one.
The difference is in how the anchor of that selection is determined:

| Concept | What the search returns | Anchor of resulting selection |
|---------|-------------------------|-------------------------------|
| Motion | a new *head* position | determined by the motion mode (may come from old selection state) |
| Text object | a run of whole characters | its start in move mode; in extend mode, *unions* with the existing selection |

The search behind a motion only answers "where does the head go?". It is
given the whole current selection, not only the head: most motions read the
head alone, but one such as matching-pair can resolve against the whole span.
In move mode
(`h`, `l`, `j`, `k`), the anchor collapses to the new head, producing a
single-character selection. In extend mode, the anchor stays fixed and only
the head moves — growing the selection.

A text object bypasses the motion mode in **move mode only**: it returns a
complete run of characters and the framework creates a fresh selection from start to end,
discarding the previous anchor. In extend mode the matched range is *united*
with the current selection instead — the new start is the earlier of the two
starts, the new end the later of the two ends, and the existing direction is
preserved. A result is always a run of whole character clusters, so a union
never leaves half a character outside. The extend variant also does an outward-growth retry: when the first
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

These are four patterns: motion, text object, word select, and structural
navigation.

## The search is separate from the selection bookkeeping

All the patterns follow the same design: the search is *pure and ignorant of
multi-cursor*. It looks at one selection and returns one result. The framework
around it handles iterating over all selections and merging any that converge.

A motion's search answers a coordinate question ("where does the cursor
go?") and returns a position. A text object's search returns a run of
characters, or nothing if no match exists at the current position. On "no
match", the existing selection is preserved — pressing inner-bracket when not
inside any brackets is a no-op.

## Auto-merge after every motion or text object

After every motion or text object, selections that share a character cluster
are automatically merged into one. This is essential for multicursor
correctness: if two cursors are both inside the same bracket pair and you press
inner-bracket, you want one combined selection, not two identical overlapping
ones. Selections that only touch, with no cluster in common, stay separate.
