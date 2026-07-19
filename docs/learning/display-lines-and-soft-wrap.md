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
calculation, so wide-tab lines behave predictably.

## Visual-line movement (`j` and `k` in wrap mode)

When wrap is off, `j` and `k` move by one buffer line — straightforward. When
wrap is on, the user expects `j`/`k` to move by one *display row*, which may
stay on the same buffer line if that line spans multiple rows. (A `j`/`k`
with an explicit count — `9j` — deliberately moves by buffer lines even in
wrap mode, so it matches relative line numbers.)

Visual-line movement needs to know the visual column of the cursor, not just
its character offset in the buffer. The goal is "land on the closest character
in the row above/below at the same visual column" — which requires knowing what
column the characters in adjacent rows start at.

To do this without re-rendering the whole frame, the engine's format stage is
called for the specific buffer line being moved into. The resulting grapheme
list is then scanned for the grapheme whose visual column is closest to the
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
