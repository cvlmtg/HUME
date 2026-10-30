# The Rendering Pipeline: Engine, Providers, and the 4-Stage Pipeline

Every frame, HUME takes a buffer full of text and produces a screen full of
styled terminal cells. This document explains the four-stage pipeline that
does that and why it is structured the way it is.

## The four stages

```
Buffer (text + config)
         │
         ▼
  Stage 1: Layout        ← "how wide are the gutter and the text area?"
         │  → pane geometry: gutter width, content width
         ▼
  Stage 2: Format        ← "where does each character appear on screen?"
         │  → one entry per display line; one entry per grapheme with its display column
         ▼
  Stage 3: Style         ← "what colour and decoration does each character get?"
         │  → one style value per grapheme
         ▼
  Stage 4: Compose       ← "write styled glyphs to the screen buffer"
         │
         ▼
  Screen buffer          ← diffed against previous frame → terminal escape codes
```

**Why four stages instead of one loop?** Each stage has a well-defined input
and output, and each is the natural extension point for one class of
feature:

- **Gutter columns** (line numbers, git signs, diagnostic icons) plug into
  Stage 4: they draw in the gutter laid out by Stage 1.
- **Syntax highlighting, search matches, bracket highlighting** plug into
  Stage 3: they add or override style on individual graphemes.
- **Virtual lines** (display lines any plugin can inject, such as diff
  context or code lenses) plug into Stage 2: they are display lines that do
  not correspond to buffer lines.
- **Floating overlays** (completion popup, hover) plug into Stage 4: they
  draw over the composed output.

Syntax highlighting, for example, is a Stage 3 concern only. It reads the
format output from Stage 2 and emits style values, and never needs to know
about layout or composition.

## What each stage produces

**Stage 1 (Layout)** computes the pane's geometry: how many columns the
gutter takes and how many are left for text. The gutter is sized for the
last line of the whole file, not the last line on screen, so its width stays
put while you scroll. Which lines are visible is not decided here: the
render walks display lines down from the viewport's top, and that walk is
the layout (see "The fused loop" below).

**Stage 2 (Format)** turns buffer text into *display lines*. A display line
is one visual line on screen: in soft-wrap mode a long buffer line produces
several (a "line start" display line plus one or more "continuation" display
lines). Each display line is a sequence of *cells*, and each cell is one of
a small set:

- a **grapheme**: a real character, with its display column and visual
  width (most are 1 column; CJK double-width characters are 2);
- an **indicator**: a glyph standing in for a tab or a whitespace run when
  the user has asked to see them;
- a **tab fill**: the blank cells a tab expands to when its indicator is
  off;
- a **placeholder**: a stand-in for a character the terminal must never be
  shown as itself, either a control character it would act on or an
  invisible one that would vanish. A bidi override is the dangerous case,
  since rendered as itself it can look like a harmless space. It is drawn as
  its codepoint in angle brackets, so nothing hides in a line that looks
  empty, and unlike the indicator it is always on;
- a **width continuation**: the empty second cell of a 2-wide character,
  which inherits its neighbour's style;
- a **virtual** cell: text injected by a provider that is not backed by
  buffer content (inlay hints, ghost text);
- an **empty** cell: the cell at the end of every line that the cursor sits
  on when it is on the newline (and the only cell of an empty line).

Virtual lines, whole display lines injected by providers, are formatted here
too. The cell vocabulary lets one rendering loop draw real text, visible
whitespace and inlay hints through the same machinery. Tilde filler lines
below the last buffer line are the one exception: they carry no content and
are painted by a small dedicated path.

A parallel extension point, inline decorations, injects cells at byte
offsets *inside* a display line rather than as separate display lines. Inlay
hints are drawn this way, inline with the code they annotate.

**Stage 3 (Style)** walks the cells and gives each a resolved style:
foreground, background, bold, italic, underline. Style comes from several
layered sources:

- the base theme
- the cursorline background, on the primary cursor's line
- syntax highlighting
- plugin-supplied highlight spans
- search matches
- LSP diagnostic underlines
- the bracket-match highlight
- the selection background
- the cursor head itself

Inline decorations carry their own colour, above every highlight layer but
below the selection. Layers are applied in priority order, each compositing
*over* the previous one: a later layer overrides only the fields it sets.
So a search match's background wins over syntax highlighting's, but on a
selection, the selection's background wins over the match's. The primary
selection and cursor are styled differently from the others, so a
multi-cursor view always shows which cursor is focused.

**Stage 4 (Compose)** writes styled text into the terminal's screen buffer:
the gutter on the left, the content area to its right. On the first display
line of each buffer line, indent guides (thin vertical rules at each inner
tab stop of the leading whitespace) keep nested blocks visually aligned.
Overlays such as the completion popup are drawn last, on top of everything.
The status line, a tab bar and the bottom drawer (used for list output such
as diagnostics) each reserve rows above or below the content instead of
covering it.

## The fused loop

The pipeline does not build every display line for the screen before
styling. It walks display lines from the viewport's top address, which can
sit partway into a long line's block of display lines, and fuses stages 2–4
into that walk:

```
start at the viewport's top display line
while the pane has rows left:
    take the next display line (a virtual line, a line start, or a continuation)
    if it starts a buffer line not formatted yet:
        format that buffer line once (Stage 2) → all of its display lines
    style this display line   (Stage 3)
    compose it to the screen  (Stage 4)
fill the rows below the last buffer line with filler (tilde lines, or blank)
```

Because the walk itself decides what is visible, no separate estimate of
"how many lines fit" can disagree with what is drawn.

Work is measured and kept one buffer line at a time, so what a frame holds
is proportional to what is on screen, not to the size of the file. The
editor measures lines before drawing them, which is how it decides where the
viewport lands, and keeps the result, so the drawing step finds that work
already done. A line it only measured costs almost nothing. The memory
behind a drawn line is reused by the next frame, and only a line far wider
than anything ordinary gives its memory back at the end of the frame, so one
runaway line cannot hold on to it for the rest of the session.

## Scope-based theming

Colours are resolved through a scope hierarchy, the same one Helix uses
(it comes from TextMate grammars). A scope is a dot-separated string like
`keyword.function` or `ui.cursor.match`, and the theme maps scope
*prefixes* to styles. Syntax scope names come from tree-sitter highlight
queries; see [Tree-sitter: Grammars, Queries, and
Plum](tree-sitter-pipeline.md) for how those are installed and applied.

A grapheme tagged `keyword.function` matches the exact `keyword.function`
scope first, then `keyword`, then the base scope, and uses the most specific
match found. A theme that only defines `keyword` therefore styles
`keyword.function` and `keyword.type` too, and its author can refine it by
adding more specific entries without touching the rest.

## The engine/editor boundary

The rendering layer is self-contained: it knows nothing about the editor's
top-level state, file loading or modes. It reads a buffer's text through a
closure that lends it the rope for the duration of the call, and reads
syntax highlighting the same way. It never owns a buffer and never depends
on editor types; it only borrows the text and syntax data for the frame.
That keeps rendering testable in isolation and avoids a circular dependency
between the editor and the renderer.

Per-frame data that crosses the boundary (search highlights, bracket match
positions, LSP diagnostics) is written by the editor into a per-pane slot
before rendering starts, and read by Stage 3 during styling. Each pane has
its own slot, so one pane's search highlights never bleed into another's.
The slot sits behind a lock that is never contended: the editor writes once
before the frame, and rendering only reads.
