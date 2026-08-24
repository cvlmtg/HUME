# Display Lines and Soft Wrap

## Why "display line" ≠ "buffer line"

A buffer line is one run of characters terminated by a newline — what you'd
call a "line" in a text file. A display line is one horizontal row on screen.
The two coincide when wrapping is disabled: each buffer line occupies exactly
one row. When soft wrap is enabled, a long buffer line may be split across two,
five, or twenty rows on screen.

HUME treats display rows as the fundamental unit of rendering from the start,
even when wrapping is off and the two are equivalent. This is an investment in
avoiding future pain: adding virtual lines (inline diagnostics, diff context,
decorations) to a renderer that iterates buffer lines directly is a significant
retrofit. Iterating display rows from the beginning makes virtual lines a
natural extension rather than a hack.

## The four kinds of display row

Each row on screen belongs to one of four categories:

| Kind | Description |
|------|-------------|
| **Line start** | The first row of a buffer line — always exists, even for a single-char line |
| **Wrapped continuation** | A subsequent row that continues a long buffer line beyond the viewport width |
| **Virtual** | A row injected by a provider, not backed by buffer text (e.g. a plugin-injected annotation row) |
| **Filler** | An empty row below the last buffer line, conventionally shown with a tilde (`~`) |

The renderer processes one buffer line at a time. For each buffer line, it
generates one "line start" row and zero or more "wrap continuation" rows,
drains any virtual rows anchored to that line, and repeats. After the last
buffer line, filler rows fill the remaining space.

## Soft wrap

When soft wrap is enabled, lines longer than the content area width are split
at grapheme boundaries. The split point respects word boundaries in "word wrap"
mode: as the format stage walks the line forward it remembers the most recent
whitespace position, and when a grapheme would overflow the right edge it
breaks at that remembered position rather than mid-word. The effect is the
same as walking back from the right edge; the cost is a single forward pass.

A second mode, *indent wrap*, starts each continuation row at the parent
line's indentation level, rounded down to whole tab stops — while still
breaking at word boundaries — so deeply nested arguments stay visually
nested across the wrap instead of snapping back to column zero.

Every continuation row knows which buffer line it belongs to (so line-number
rendering and selection highlighting still work) and which sub-row within that
line it is (so double-clicking or the cursor position can identify the right
character).

Tab characters are expanded to the nearest tab stop *before* the wrap
calculation, so wide-tab lines behave predictably. That width is fixed at
the column the tab was first measured against — if wrapping later moves it
to a continuation row and renumbers its column from zero, the tab keeps its
original width rather than re-expanding to whatever a fresh tab stop at the
new column would give.

## Visual-line movement (`j` and `k` in wrap mode)

When wrap is off, `j` and `k` move by one buffer line — straightforward. When
wrap is on, the user expects `j`/`k` to move by one *display row*, which may
stay on the same buffer line if that line spans multiple rows. (A `j`/`k`
with an explicit count — `9j` — deliberately moves by buffer lines even in
wrap mode, so it matches relative line numbers.)

Visual-line movement needs to know the display column of the cursor, not just
its character offset in the buffer. The goal is "land on the closest character
in the row above/below at the same display column" — which requires knowing what
column the characters in adjacent rows start at.

To do this without re-rendering the whole frame, the engine's format stage is
called for the specific buffer line being moved into. The resulting grapheme
list is then scanned for the grapheme whose display column is closest to the
target column.

### The sticky column

When a user presses `j` three times in a row, the third press should try to
land at the same column as the first. Without a sticky column, each press would
calculate the "current column" from the cursor's new position, and the column
would drift leftward if some intermediate rows are shorter than the original.

HUME latches the target column on the first `j` or `k` press and holds it
across consecutive vertical moves. Any horizontal movement (or any non-vertical
command) resets the latch. This matches how virtually every text editor handles
vertical movement with short lines.

The same guarantee applies to a counted move like `9j`: the column it started
with survives even if a narrower line sits somewhere in the middle of the
nine-line hop, exactly as if `j` had been pressed nine times in a row.

There is one subtlety a counted move has to get right that a single press
doesn't: while wrapping, a display row's column is measured from that row's
own left edge, not from the start of the buffer line it belongs to — a
continuation row that starts a few columns in renumbers its columns from
zero. So the column a bare `j` latches while hopping between display rows and
the column a counted move latches while hopping between buffer lines are two
different numbers for the same character whenever a line wraps. HUME tracks
which of the two a latched column was measured in, and only reuses it for a
move that counts the same way — a move that counts differently re-measures
the column from where the cursor actually sits rather than misreading the
other kind of column as its own. With wrapping off a display row and a buffer
line are the same thing, so the two always agree and a mixed run of plain and
counted vertical moves keeps its column throughout.

## Connection to LSP and future features

Inline decorations — a mechanism that injects cells *inside* a row at byte
offsets rather than as separate rows — carry both inlay hints and the
end-of-line diagnostic summary, sitting inline with the code they annotate
rather than on their own row. Virtual rows are the complementary,
whole-row-granularity mechanism, available to any plugin that wants to
anchor an annotation row below a buffer line without disrupting the buffer
text or changing any line numbers.

The display-line abstraction is also the mechanism by which syntax-highlighted
folding ("fold this function to one line") would eventually work: a fold
would suppress all but the "line start" row of the folded range, with one
virtual row substituting a summary.

---

*See also: [The Rendering Pipeline](rendering-pipeline.md) for how the display
row stream is styled and composed into terminal output.*
