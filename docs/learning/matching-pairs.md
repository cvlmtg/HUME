# Matching Pairs: Depth Tracking and Two Accepted Limitations

## What it does, and why it isn't `%`

Pressing `#` jumps from a bracket to its partner, or from an HTML/XML/JSX tag
to its partner — the equivalent of Vim's `%`. HUME uses `#` instead because
`%` already means select-all, and `g m` was already claimed by a picker.
Watch for the muscle-memory trap if you're coming from Vim.

Tag matching fires from anywhere inside the tag's own markup — the `<`, the
`>`, the tag name, or an attribute — not just the two delimiter characters.
That matches Vim's `matchit` plugin rather than the bare `%` built-in, which
only fires from a delimiter itself.

## Depth tracking, and scanning outward from the cursor

Brackets have distinct open and close characters, which is what makes a
depth-tracking scan possible: walk the text, add one to a depth counter on
every open character, subtract one on every close, and stop the moment depth
returns to zero. That zero is unambiguously *this* bracket's partner, however
deeply the pair is nested inside others. (Quote characters can't do this —
see [Quote Scanning](quote-scanning.md) for why they need a different
algorithm entirely.) The same technique matches a `<tag>` against its
`</tag>`, tracking tag depth by name instead of a single counter.

Depth tracking has a property worth naming: it's *direction-symmetric*.
Scanning outward from the cursor toward a bracket's partner gives the same
answer as scanning the same pair from the start of the buffer forward, just
by starting the depth count somewhere other than zero. Matching didn't
always take advantage of this — tag matching originally reparsed the entire
buffer from the top on every `#` press, discarding all of that work as soon
as the answer was found. Scanning outward from the cursor instead touches
only the handful of tags actually between the cursor and its partner, with
no loss of correctness.

## `<>` is deliberately not a bracket pair

The set of bracket pairs `#` matches is `()`, `[]`, and `{}` — `<>` is
conspicuously absent. That isn't an oversight: in real code, `<` is a
comparison operator (`a < b`) far more often than it's a delimiter, and a
generic type like `Vec<String>` would false-match constantly if `<>` were
treated as a bracket pair. Vim's own default `matchpairs` setting excludes
`<>` for the same reason. Tag matching handles `<div>`/`</div>` through its
own separate scan, so nothing is lost — `<>` just isn't *also* claimed by
the generic bracket table.

## One resolver, two consumers

The bracket highlighted under the cursor and the bracket `#` actually jumps
to come from the same resolver. That wasn't always true: the highlight used
to have its own inline copy of the bracket-matching logic, including its own
copy of the pair table — one that, unlike the real one, included `<>`. The
result was an editor that highlighted a `<>` pair as a matched bracket while
`#` refused to jump to it, because the two copies had quietly drifted apart.
Collapsing both consumers onto one shared resolver doesn't just remove
duplication — it makes that class of drift structurally impossible, since
there's only one place left for the pair table to be read from.

## Resolving against the whole selection

`#` doesn't just look at the character under the cursor — it looks at the
whole current selection. This matters because HUME's word motions
deliberately select trailing whitespace: a `w` motion landing on `") "`
places the cursor on the space *after* the closing paren, not on the paren
itself. A resolver that only checked the character under the cursor would
find nothing there and treat `#` as a no-op, even though a bracket is
sitting right at the edge of the selection.

The fix is to search the selection itself. The cursor (the selection's
*head*) is always at one end of the selection, never in the middle, so
"nearest bracket, from the head" is just "scan the selection from the head's
end inward" — the first bracket found is, by construction, the nearest one.
This search is bounded by the selection: a bracket sitting just past the
selection's edge is deliberately not found, even if it would otherwise be
the nearest one in the buffer.

## Accepted limitation: the search is capped at one line

The same resolver that answers "where does `#` jump to" also drives the
cursor-match highlight, and the highlight recomputes every single frame —
not just on a keypress. A selection can be made arbitrarily large (select-all
is one keystroke away), and scanning an arbitrarily large selection on every
frame would mean an arbitrarily large amount of work for something that
merely has to redraw.

So the whole-selection search only runs when the selection sits on a single
line. A selection spanning multiple lines falls back to checking just the
cursor's own character. This is narrower than the full search, but it costs
nothing in practice: the whitespace HUME's word motions select never crosses
a line boundary, so the exact case the whole-selection search exists for —
a bracket left just behind a selected space — still works. The cap only
bites a multi-line selection whose bracket isn't under the cursor itself; a
multi-line selection with the cursor already on a bracket matches normally,
since that's just the single-character fallback succeeding on its own.

## Brackets and tags are resolved asymmetrically, on purpose

Bracket resolution searches the whole selection; tag resolution only ever
checks the cursor itself. That's not an oversight either — it's a cost
trade the other direction. A bracket lookup at a single position is cheap.
A backward tag lookup is not: finding "the nearest enclosing tag going
backward from here" is unbounded work at *each* position it's tried from,
so probing every position across a selection would multiply that unbounded
cost by the selection's length. Brackets are also tried before tags for the
same reason in miniature — ruling out a bracket match is cheap, so it's
worth doing first.

The same cost keeps tag matching out of the per-frame highlight entirely.
The highlight only ever shows a matched bracket; `#` still jumps to a tag's
partner correctly, it's just never drawn as a highlight. When two things
that look like they should behave identically actually have very different
costs to compute, giving them different rules is the right call, not a
missed unification.

## Accepted limitation: a stray `<` inside an enclosing tag

Backward tag matching finds candidate `<` characters walking left, then
parses forward from each one — reusing the *forward* parser rather than
writing a second, mirror-image parser that would have to un-read quoted
attributes and braced JSX expressions in reverse. That reuse has one real
gap: a `<` that lives inside an *enclosing* tag's own quoted attribute or
JSX brace — `title="</div>"`, or `<!-- <div> -->` — is invisible to a
forward parse (the enclosing tag's own parser consumes it whole, as
harmless text), but can itself look like a well-formed tag when the same
text is found by scanning backward. That miscounts the matching depth.

In practice this only misfires when the buried fragment is *unbalanced* — a
single tag-shaped construct with no matching partner of its own inside the
same attribute. A balanced pair inside the attribute cancels itself out and
causes no harm. This is accepted as a known limitation of the scan rather
than fixed with a more elaborate parser.

## The lexical scan today, and tree-sitter tomorrow

The scan described above is a fallback: tree-sitter-backed tag matching is
expected to be built, and once it lands it should take over matching in a
buffer with a grammar loaded — a real syntax tree already knows which
characters are inside an attribute string, so it doesn't have the buried-`<`
limitation above at all. The lexical scan will stay as the path for
everything a tree-sitter matcher can't cover: a buffer with no language
configured, a scratch buffer, or any file whose language has no grammar
installed. Which buffers fall into which bucket, and exactly how the two
would hand off, is still an open design question, not yet decided.

Tree-sitter tag matching isn't merely unwritten, though — it can't be built
in the same place the lexical scan lives. The code that implements motions
sits at a lower architectural layer than tree-sitter integration, and a
lower layer can't depend on a higher one. So a tree-sitter-backed matcher
will have to be added somewhere else in the codebase entirely, not dropped
in beside the lexical scan as an alternate branch — one more reason the two
paths are expected to coexist rather than one simply replacing the other.

## The motion is its own inverse

`#` doesn't take a count the way most motions do. Vim's `%` interprets a
count as "go to N percent of the file" — a different feature entirely, one
HUME doesn't implement — so a count on `#` was given no special meaning
rather than being pressed into that role by accident. This also follows
naturally from what the motion actually is: pressing `#` twice in a row
returns the cursor to exactly where it started, since a bracket's partner's
partner is the original bracket. A motion that's its own inverse has no
useful notion of "do it three times" beyond "do it once" (odd) or "do
nothing" (even), so ignoring the count is the only interpretation that
doesn't invent a meaning nobody asked for.

## One more grapheme-cluster edge

As covered in
[Unicode Position Model](unicode-position-model.md), some codepoints join
*forward* into the following character rather than standing alone. A
bracket character itself is always plain ASCII, but a codepoint immediately
before one can still join into it — landing the cursor on that leading
codepoint rather than on the bracket's own character. If the resolver
checked only that exact character, this would break the "own inverse"
property from a very specific starting position: a second `#` press would
find nothing, and the highlight would go dark on a bracket the cursor is
visibly sitting next to. The resolver checks the whole cluster the cursor
is on, not a single character, specifically to avoid that gap.
