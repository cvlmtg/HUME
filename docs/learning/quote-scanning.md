# Quote Scanning: Parity Instead of Depth

## Why quotes are different from brackets

Brackets use distinct open and close characters (`(` vs `)`), which allows a
depth-tracking scan: increment depth on open, decrement on close, stop at
depth zero. This handles nesting correctly.

Quotes use the *same* character for open and close (`"..."`, `'...'`). There
is no depth to track, and nesting is ambiguous anyway (most languages do not
allow nested same-character quotes). A different algorithm is needed.

## The parity scan

The quote scanner walks the current line and uses parity to assign roles:

```
Position:  0   1   2   3   4   5   6   7
           "   h   e   l   l   o   "   !
           ↑                       ↑
         odd (open)              even (close)
```

The scan goes left to right. Each quote character it finds alternates
between "opening" (odd occurrence) and "closing" (even occurrence). When a
complete pair is found, the scan checks whether the cursor falls inside it,
on either quote included. If it does, the pair is returned; if not, the pair
is discarded and the search continues with the next pair.

A cursor *on* a quote character is handled by the same parity logic: whether
that quote is the opener or the closer depends on how many quotes precede it
on the line.

## Why brackets can span lines but quotes cannot

Bracket text objects walk the whole buffer and track depth. They work across
lines because `(` and `)` are asymmetric: the scanner can always tell which
direction it is going. See [Matching Pairs](matching-pairs.md) for how the
same asymmetry is used to jump between a bracket and its partner.

The parity scan cannot be extended to several lines with the same
guarantee. Suppose line 1 has an unmatched `"` (a string that continues on
the next line, an apostrophe in a comment, a stray character). The parity
count is then off by one for every line that follows: every pair assignment
downstream is inverted, and the text object selects the wrong region.

So quote text objects are bounded to the cursor's line. This applies to all
three quote characters, backticks included: a backtick text object does not
reach across a multiline inline code span in Markdown (CommonMark allows
`` `foo\nbar` ``). The same scan backs the surround family (`ms"`, `ms'`,
``ms` ``), so it has the same line bound.

A syntax tree removes the ambiguity, because it knows which quote
characters open a string literal. Resolving quote objects against the tree
when a grammar is loaded is on the roadmap; today the line-bounded parity
scan is the only quote text object.

## Empty pairs

An empty pair (`""`, `''`, ``` `` ```) has no inner range in the
inclusive-selection model, so `mi"` inside `""` is a no-op rather than a
zero-width selection.

---

*See also: [Matching Pairs](matching-pairs.md) for the depth-tracking scan
brackets use; [Tree-sitter: Grammars, Queries, and Plum](tree-sitter-pipeline.md)
for syntax trees.*
