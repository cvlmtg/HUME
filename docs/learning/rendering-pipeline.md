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
- **Virtual lines** (inline diagnostics, diff context) plug into Stage 2 —
  they inject rows that don't correspond to buffer lines.
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
rows). Each display row is a sequence of grapheme entries, each annotated with
its visual column, visual width (most characters are 1 column wide; CJK
double-width characters are 2), and character class. Virtual lines (inserted
by providers, not backed by buffer text) also appear here.

**Stage 3 (Style)** walks the grapheme entries and assigns a resolved style to
each one — foreground colour, background colour, bold/italic/underline. Style
comes from multiple layered sources: the base theme, syntax highlighting spans,
selection highlighting, search match highlighting. Sources are checked in
priority order; the highest-priority source that covers a grapheme wins.

**Stage 4 (Compose)** writes styled grapheme text into the terminal's screen
buffer. Gutter columns are drawn to the left, the content area to the right.
Overlays (like the completion popup) are drawn last, on top of everything else.

## The fused loop

Instead of materialising all rows for the full visible range before styling, the
pipeline fuses stages 2–4 into a single loop over buffer lines:

```
for each buffer line in the visible range:
    drain any virtual lines anchored above this line   ← virtual content first
    for each display row of this line:
        format this row (Stage 2)
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

The engine crate is self-contained — it knows nothing about the editor's
top-level state, file loading, or modes. It receives a buffer's text via a
closure that returns the buffer's text, rather than by owning or borrowing
the buffer directly. This keeps the engine testable in isolation and prevents
a circular dependency between the editor layer and the rendering layer.

Per-frame data that crosses the boundary (search highlights, bracket match
positions) is written by the editor into a shared slot before rendering begins,
then read by the engine's Stage 3 providers during styling. The editor writes
once per frame; the engine reads once per frame. No locking is needed during
the render itself.
