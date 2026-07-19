# The Rendering Pipeline: Engine, Providers, and the 4-Stage Pipeline

Every frame, HUME takes a buffer full of text and produces a screen full of
styled terminal cells. This document explains the four-stage pipeline that does
that, how the design accommodates future features, and why the pipeline is
structured the way it is.

## The four stages

```
Buffer (text + config)
         │
         ▼
  Stage 1: Layout        ← "which lines are visible? how wide is the gutter?"
         │  → visible line range, column widths
         ▼
  Stage 2: Format        ← "where does each character appear on screen?"
         │  → one display row per visual line; one entry per grapheme with its column
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
and output, and each one is the natural extension point for a distinct class of
feature:

- **Gutter columns** (line numbers, git signs, diagnostic icons) plug into
  Stage 4 — they draw in the gutter columns laid out by Stage 1.
- **Syntax highlighting, search matches, bracket highlighting** plug into
  Stage 3 — they add or override style on individual graphemes.
- **Virtual lines** (a general mechanism any plugin can inject rows through —
  diff context, code lenses) plug into Stage 2 — they inject rows that don't
  correspond to buffer lines.
- **Floating overlays** (completion popup, hover) plug into Stage 4 — they
  draw over the composed output.

Adding syntax highlighting, for example, is a Stage 3 concern only. It reads
the format output from Stage 2 and emits style values. It doesn't need to know
about layout or composition — those stages are untouched.

## What each stage produces

**Stage 1 (Layout)** computes which buffer lines fall in the visible viewport
and how much horizontal space the gutter occupies. The output is a visible
range plus per-column widths.

**Stage 2 (Format)** turns buffer text into *display rows*. One display row
is one visual line on screen — in soft-wrap mode, a long buffer line produces
multiple display rows (a "line start" row plus one or more "continuation"
rows). Each display row is a sequence of *cells*, and each cell is one of a
small set:

- a **grapheme** — a real character, annotated with its visual column and
  visual width (most are 1 column; CJK double-width characters are 2);
- an **indicator** — a placeholder glyph standing in for a tab or a
  whitespace run when the user has asked to see them;
- a **width continuation** — the empty second cell of a 2-wide CJK character,
  which inherits its neighbour's style;
- a **virtual** cell — text injected by a provider that isn't backed by
  buffer content, the in-row cousin of a virtual row (inlay hints, ghost text);
- an **empty** cell — a placeholder at the end of every line, so the cursor
  has a cell to land on when it sits on the newline (and the sole cell of an
  empty line).

Virtual rows (whole rows injected by providers, not backed by buffer text)
also appear here. The cell vocabulary is what lets a single rendering loop
draw real text, visible whitespace, and inlay hints through the same
machinery. (Tilde filler rows below the last buffer line are the one
exception — they carry no content at all and are painted by a small
dedicated path.)

A parallel extension point — inline decorations — injects cells at byte
offsets *inside* a row rather than as separate rows. This is how inlay hints
are rendered, sitting inline with the code they annotate.

**Stage 3 (Style)** walks the cells and assigns a resolved style to
each one — foreground colour, background colour, bold/italic/underline. Style
comes from multiple layered sources: the base theme, the cursorline background
(applied to the primary cursor's line), syntax highlighting spans,
plugin-supplied highlight spans, search match highlighting, LSP diagnostic
underlines, bracket-match highlighting, the selection background, and the
cursor head itself. Inline decorations carry their own colour, above every
highlight layer but below the selection. Layers are
applied in priority order, each compositing *over* the previous one — a later
layer overrides only the fields it sets, leaving the rest intact. So a search
match's background wins over syntax highlighting's, but if the match sits on a
selection, the selection's own background wins over the match's. The primary
selection and primary cursor are styled distinctly from their secondary
counterparts, so a multi-cursor view always shows which cursor is focused.

**Stage 4 (Compose)** writes styled grapheme text into the terminal's screen
buffer. Gutter columns are drawn to the left, the content area to the right.
On the first row of every buffer line, indent guides — thin vertical rules at
each inner tab stop of the leading whitespace — are drawn so nested blocks
stay visually aligned even when the user has tabs turning into spaces or vice
versa. Overlays (like the completion popup) are drawn last, on top of
everything else; the status line, any tab bar, and the collapsible bottom
drawer (used for list-style output such as diagnostics) each claim reserved
rows above or below the content rather than overlaying it.

## The fused loop

Instead of materialising all rows for the full visible range before styling, the
pipeline fuses stages 2–4 into a single loop over buffer lines:

```
for each buffer line in the visible range:
    drain any virtual lines anchored above this line   ← virtual content first
    format this buffer line once (Stage 2)              → produces all its display rows
    for each display row of this line:
        style this row  (Stage 3)
        compose to screen (Stage 4)
    drain any virtual lines anchored below this line
fill remaining rows with empty filler (tilde rows, or blank)
```

Processing one buffer line at a time keeps peak memory proportional to the
width of one line, not the height of the viewport. The scratch buffers that hold
display rows, grapheme entries, and style values are reused across frames —
they grow to their steady-state size after a few frames and then cause no
further allocation.

## Scope-based theming

Colours are resolved through a scope hierarchy (inspired by TextMate grammars,
identical to Helix). A scope is a dot-separated string like
`keyword.function` or `ui.cursor.match`. The theme maps scope *prefixes* to
style values. Syntax scope names come from tree-sitter highlight queries; see
[Tree-sitter: Grammars, Queries, and Plum](tree-sitter-pipeline.md) for how
grammars and queries are installed and applied.

A grapheme tagged with `keyword.function` will match first
against the exact `keyword.function` scope, then `keyword`, then the base
scope — using the most specific match found.

This means a theme that only defines `keyword` automatically styles all more
specific scopes like `keyword.function` and `keyword.type`, and a theme author
can progressively refine by adding more specific entries without touching the
rest.

## The engine/editor boundary

The rendering layer is self-contained — it knows nothing about the editor's
top-level state, file loading, or modes. It fetches a buffer's text through a
closure that lends it the rope for the duration of the call, and it fetches
syntax highlighting the same way: a closure handed in for the frame, not a
value the renderer owns. It never *owns* a buffer and never depends on
editor-domain types, but it does borrow both the rope and the syntax data
through those closures. This keeps rendering testable in isolation and
prevents a circular dependency between the editor and the renderer.

Per-frame data that crosses the boundary (search highlights, bracket match
positions, LSP diagnostics) is written by the editor into a shared per-pane
slot before rendering begins, then read by Stage 3's providers during
styling. The editor writes once per frame; rendering reads once per frame.
Each pane has its own slot, so one pane's search highlights never bleed into
another's. The slot is wrapped in a lock, but the lock is uncontended — one
write outside the render, then reads inside it — so the cost is a few
nanoseconds per read rather than a true contention tax.
