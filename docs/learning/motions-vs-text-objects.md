# Motions vs Text Objects

## The conceptual split

Both motions and text objects look at the current selection and produce a
new one. The difference is in how the anchor of that selection is determined:

| Concept | What the search returns | Anchor of resulting selection |
|---------|-------------------------|-------------------------------|
| Motion | a new *head* position | determined by the motion mode (may come from old selection state) |
| Text object | a run of whole characters | its start in move mode; in extend mode, *unions* with the existing selection |

The search behind a motion only answers "where does the head go?". It is
given the whole current selection, not only the head: most motions read the
head alone, but one such as matching-pair can resolve against the whole span.
In move mode (`h`, `l`, `j`, `k`), the anchor collapses to the new head,
producing a single-character selection. In extend mode, the anchor stays
fixed and only the head moves, growing the selection.

A text object bypasses the motion mode in move mode only: it returns a
complete run of characters and the framework creates a fresh selection from
start to end, discarding the previous anchor. In extend mode the matched
range is *united* with the current selection instead. The new start is the
earlier of the two starts, the new end the later of the two ends, and the
existing direction is preserved. A result is always a run of whole character
clusters, so a union never leaves half a character outside. The extend
variant also retries outward: when the first match is a subset of what is
already selected, it resumes the search from one past the current end, so
repeated extend-mode bracket and quote objects climb to the enclosing pair
rather than re-reporting the inner one.

## Four patterns

Motions and text objects are two of four patterns for building a selection:

| Pattern | Returns | Anchor (in move mode) | Typical use |
|---|---|---|---|
| Motion | a new cursor position | the new head | `h/j/k/l`, goto-line |
| Text object | an optional run of characters | the run's start | `miw`, `mi(`, `mi"` |
| Word select | an optional run of characters | the run's start | `w/b/W/B` |
| Structural navigation | an optional run of characters | the run's start | `{`/`}`, next function |

Word motions (`w`/`b`/`W`/`B`) sit between the first two: navigational like
motions, but returning a full word range. They are described in
[Word Motions](word-motions.md).

Structural navigation combines pieces of the other three. It steps to the
next or previous function, class, comment, test, argument, value (an array,
tuple or struct literal) or tag pair, and the paragraph motions (`{`/`}`)
follow the same pattern. Like a text object, each step returns a whole
object span rather than a coordinate. Like a motion, it is a repeatable,
count-driven search that can no-op: pressing it past the last object in the
buffer leaves the selection where it was.

Its extend mode borrows the text object's growth rule rather than the plain
motion one. Instead of pinning the anchor and moving only the head, each
further press *unites* the newly found object with what is already selected,
so growing across several objects in a row, or over one nested inside the
one just selected, never loses ground already covered.

The pattern does not care how the object was found, only that a whole span
comes back. Most kinds come from a tree-sitter query. The paragraph motions
come from a lexical scan for blank-line boundaries, and tag pairs use the
grammar when it defines them and a lexical scan otherwise.

## The search is separate from the selection bookkeeping

All four patterns follow the same design: the search is pure and ignorant of
multi-cursor. It looks at one selection and returns one result. The
framework around it iterates over all selections and merges any that
converge.

A motion's search answers a coordinate question ("where does the cursor
go?") and returns a position. A text object's search returns a run of
characters, or nothing if no match exists at the current position. On "no
match", the existing selection is preserved: pressing inner-bracket when not
inside any brackets is a no-op.

## Auto-merge after every motion or text object

After every motion or text object, selections that share a character cluster
are merged into one. This is essential for multicursor correctness: if two
cursors are both inside the same bracket pair and you press inner-bracket,
you want one combined selection, not two identical overlapping ones.
Selections that only touch, with no cluster in common, stay separate.

---

*See also: [Move vs Extend](motion-mode.md) for how the mode shapes a
motion's result; [Inner vs Around](inner-vs-around.md) for the text object
convention.*
