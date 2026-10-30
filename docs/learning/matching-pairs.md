# Matching Pairs: Depth Tracking and Two Accepted Limitations

## What it does

Pressing `#` jumps from a bracket to its partner, or from an HTML/XML/JSX tag
to its partner. It fires from anywhere inside a tag's own markup (the `<`,
the `>`, the tag name, or an attribute), not only from the delimiters. It is
a single key because it is pressed at motion frequency and names a
relationship to where the cursor is rather than a destination.

## Depth tracking, scanning outward from the cursor

Brackets have distinct open and close characters, which makes a depth scan
possible: walk the text, add one on every open character, subtract one on
every close, and stop when depth returns to zero. That zero is this
bracket's partner, however deeply the pair is nested. (Quote characters
cannot do this; see [Quote Scanning](quote-scanning.md).) The same technique
matches a `<tag>` against its `</tag>`, tracking depth by tag name.

Depth tracking is direction-symmetric: scanning outward from the cursor
gives the same answer as scanning from the start of the buffer, by starting
the count somewhere other than zero. Tag matching relies on this. Rather
than parsing the buffer from the top on every press, it scans outward from
the cursor and touches only the tags between the cursor and its partner.

## `<>` is not a bracket pair

`#` matches `()`, `[]` and `{}`. In real code `<` is a comparison operator
(`a < b`) far more often than a delimiter, and a generic type like
`Vec<String>` would false-match constantly if `<>` were a bracket pair. Tags
(`<div>`/`</div>`) have their own scan, so nothing is lost.

## Resolving against the whole selection

The bracket highlighted under the cursor and the bracket `#` jumps to come
from one resolver with one pair table, so the two always agree.

The resolver looks at the whole selection, not only the character under
the cursor, because a word selection can end on whitespace. A word with no
whitespace before it takes the whitespace after it (see
[Inner vs Around](inner-vs-around.md#the-whitespace-rule-for-mawmaw)), so in
`foo(a) bar` a `w` that lands on the `)` selects `) ` and leaves the head on
the space. A resolver that only checked the head would find nothing there.

The head is always at one end of the selection, so "the nearest bracket
from the head" means scanning the selection from the head's end inward. The
search stays inside the selection: a bracket just past its edge is not
found.

## Accepted limitation: the search is capped at one line

The highlight is recomputed every frame, and a selection can be arbitrarily
large (select-all is one key away). So the whole-selection search runs only
when the selection sits on one line; a selection over several lines checks
only the cursor's own character. This loses little: the whitespace a word
selection takes never crosses a line, and a multi-line selection with the
cursor on a bracket still matches.

## Brackets and tags are resolved differently

Bracket resolution searches the whole selection; tag resolution checks only
the cursor. A bracket lookup at one position is cheap. A backward tag lookup
("the nearest enclosing tag before here") is unbounded work at each position
it is tried from, so probing every position of a selection would multiply
that cost by the selection's length. Brackets are tried before tags for the
same reason: ruling out a bracket is cheap.

The same cost keeps tags out of the per-frame highlight: it only shows a
matched bracket, while `#` still jumps between tags.

## Accepted limitation: a stray `<` inside an enclosing tag

Backward tag matching finds candidate `<` characters walking left, then
parses forward from each one. Reusing the forward parser avoids a mirrored
parser that would have to un-read quoted attributes and JSX braces in
reverse. The reuse has one gap: a `<` inside an *enclosing* tag's quoted
attribute or JSX brace, as in `title="</div>"`, is consumed whole by a
forward parse, but found by the backward walk it can look like a real tag,
and the depth count goes wrong.

It misfires only when the buried fragment is unbalanced, a single tag-shaped
construct with no partner of its own; a balanced pair cancels itself out. A
comment that is never closed has the same gap, while a closed comment is
skipped whole.

## Lexical scan and tree-sitter

A syntax tree already knows which characters sit inside an attribute
string, so a grammar-backed match has none of the gap above. The tag text
objects and the next/previous-tag motions already use it: they ask the
grammar for the element when it defines one, and fall back to the lexical
scan otherwise. `#` uses the lexical scan in every buffer. Whether it should
also prefer the grammar is an open design question.

## The motion is its own inverse

`#` ignores a count. Pressing it twice returns the cursor to where it
started, since a bracket's partner's partner is the original bracket. A
motion that is its own inverse has no useful "do it three times" beyond "do
it once" (odd) or "do nothing" (even), so ignoring the count is the only
reading that does not invent a meaning.

## One grapheme-cluster edge

A few codepoints (prepended marks such as the Arabic number sign, U+0600)
join *forward* into the character after them, so a cluster can start just
before a bracket and contain it. The cursor then sits on that leading
codepoint, not on the bracket. If the resolver checked only that one
character, a second `#` would find nothing and the highlight would go dark
on a bracket the cursor is visibly on. The resolver checks the whole cluster
the cursor is on.

---

*See also: [Quote Scanning](quote-scanning.md) for same-character pairs;
[Unicode Position Model](unicode-position-model.md) for grapheme clusters.*
