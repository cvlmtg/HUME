# Display Lines and Soft Wrap

## Why "display line" ≠ "buffer line"

A buffer line is one run of characters ended by a newline: what you would
call a "line" in a text file. A display line is one horizontal line on
screen. With wrapping off the two coincide, each buffer line taking one
display line. With soft wrap on, a long buffer line can take two, five or
twenty display lines.

HUME treats the display line as the unit of rendering even when wrapping is
off and the two are the same. A renderer that walks buffer lines has no
place to put a line that is not in the buffer; one that walks display lines
gets virtual lines (diagnostics, diff context, annotations) as one more kind
of display line.

## The four kinds of display line

| Kind | Description |
|------|-------------|
| **Line start** | The first display line of a buffer line; always exists, even for a single-char line |
| **Wrapped continuation** | A later display line that continues a long buffer line past the viewport width |
| **Virtual** | A display line injected by a provider, not backed by buffer text (e.g. a plugin's annotation line) |
| **Filler** | An empty display line below the last buffer line, conventionally shown with a tilde (`~`) |

Each buffer line owns a block of display lines: the virtual lines anchored
before it, its line start, its continuations, then the virtual lines
anchored after it. The renderer walks these blocks in order, and after the
last buffer line, filler lines fill the remaining space.

## Soft wrap

With soft wrap on, lines longer than the content area are split at grapheme
boundaries. In *word wrap* mode the split also respects word boundaries: as
the format stage walks the line it remembers the most recent whitespace, and
when a grapheme would overflow the right edge it breaks there rather than
mid-word. The effect is that of walking back from the right edge, at the
cost of a single forward pass.

*Indent wrap* also breaks at word boundaries, and starts each continuation
at the line's own indentation, rounded down to whole tab stops, so nested
code stays visually nested across the wrap instead of snapping back to
column zero.

Every continuation knows which buffer line it belongs to (so line numbers
and selection highlighting still work) and which piece of that line it is
(so a mouse click or the cursor can find the right character).

Tabs are expanded to the next tab stop *before* wrapping is computed, so
lines with wide tabs behave predictably. A tab keeps the width it was first
measured with: if wrapping moves it onto a continuation, where columns
restart from zero, it does not re-expand to fit a fresh tab stop there.

## Visual-line movement (`j` and `k` in wrap mode)

With wrap off, `j` and `k` move by one buffer line. With wrap on, they move
by one *display line*, which may stay on the same buffer line if it spans
several. A `j`/`k` with an explicit count (`9j`) still moves by buffer
lines, so it matches relative line numbers.

Visual-line movement needs the cursor's display column, not just its
character offset: the goal is the character in the display line above or
below that sits closest to the same column. To find it without re-rendering
the frame, the engine formats just the buffer line being moved into and
scans its graphemes for the one nearest the target column.

### The sticky column

Press `j` three times in a row and the third press should still aim for the
column of the first. If each press measured the column from where the cursor
had just landed, the column would drift left every time an intermediate line
was shorter.

HUME latches the target column on the first `j` or `k` and holds it across
consecutive vertical moves. Any other command resets the latch. A counted
move like `9j` keeps its column too, even past a short line in the middle of
the hop.

With wrapping on there is a catch. A continuation display line numbers its
columns from its own left edge, not from the start of its buffer line. So
the column a bare `j` latches (moving by display lines) and the column a
counted move latches (moving by buffer lines) are different numbers for the
same character. HUME records which kind of column it latched, and a move of
the other kind measures afresh from the cursor instead of misreading it.
With wrapping off the two kinds agree, so any mix of plain and counted moves
keeps its column.

## Inline decorations and virtual lines

Inline decorations inject cells *inside* a display line, at byte offsets,
rather than adding display lines. Inlay hints and the end-of-line diagnostic
summary are drawn this way, inline with the code they annotate. Virtual
lines are the whole-line counterpart: any plugin can anchor an annotation
line before or after a buffer line without changing the buffer text or any
line number.

---

*See also: [The Rendering Pipeline](rendering-pipeline.md) for how the
display line stream is styled and composed into terminal output.*
