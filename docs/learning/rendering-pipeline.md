# The Rendering Pipeline: ViewState → Formatter → Renderer

Every frame, HUME takes a buffer full of text and produces a screen full of
styled terminal cells. This document explains the three-layer pipeline that
does that, the key data structures, and how the design accommodates future
features like syntax highlighting, diagnostics, and virtual lines.

## The three layers

```
Buffer + ViewState
       │
       ▼
  DocumentFormatter        ← "where does each char appear on screen?"
       │  yields VisualRow per visual row
       ▼
    Renderer               ← "what does each cell look like?"
       │  writes styled cells
       ▼
  ratatui ScreenBuf        ← diffed against previous frame → terminal escape codes
```

Each layer has a single, well-bounded job:

| Layer | Question answered | Output |
|---|---|---|
| `DocumentFormatter` | Which buffer chars appear on which screen row? | `VisualRow` per row |
| Renderer | What style does each cell get? | Styled cells in `ScreenBuf` |
| ratatui | Which cells changed since last frame? | Terminal escape codes |

The renderer never re-implements line wrapping. The formatter never touches
colors. The clean boundary between them is the `VisualRow` struct.

### Why this boundary?

The formatter answers a purely geometric question: given this buffer content
and these viewport settings, which chars land on which screen row? The answer
depends only on text content, line lengths, tab widths, and wrap settings — not
on what any character looks like.

The renderer answers a purely visual question: given that this char is at this
position on screen, what color and style should it have? The answer depends on
cursor position, selections, highlights, and theme — not on how lines were
broken.

Keeping these questions separate means each can change independently. Swap the
wrap algorithm: only the formatter changes. Redesign the theme or add syntax
highlighting: only the renderer changes. And crucially, both the renderer *and*
the cursor-position mapper consume the formatter — because there is only one
place that decides row boundaries, they can never disagree about where a
character lives on screen.

### Separation within the renderer

The renderer itself has a further three-way split inside `render_row_content`:

- **What to draw** — the grapheme walk with viewport clipping (left/right
  column offsets, CJK double-width straddling). This loop drives everything.
- **How to style it** — delegated entirely to `resolve_style`, which is a pure
  function of character position and editor state, with no knowledge of
  terminals or cell layout.
- **How to render it** — delegated to `draw_cell` and `render_eol`, which
  handle the mechanics of writing one grapheme into `ScreenBuf` (tab expansion,
  whitespace substitution, CJK width).

Adding a new style layer (e.g. syntax highlighting) means touching only
`resolve_style`. Adding a new visual representation (e.g. a squiggle under a
diagnostic) means touching only `draw_cell`. Neither change touches the
grapheme walk.

### Data vs styling

`VisualRow` carries only geometry: char ranges, column offsets, wrap metadata.
It has no color, no style, no knowledge of themes. All appearance decisions
live in two dedicated structures: `EditorColors` (the theme — maps semantic
slot names to ratatui `Style` values) and `HighlightMap` (per-frame highlight
ranges with associated kinds). The renderer maps from geometry + editor state
to appearance; it never conflates the two.

---

## ViewState: what the viewport knows

`ViewState` (`src/ui/view.rs`) is the editor's model of how to display a
document. It carries three kinds of data:

**Scroll state** — which part of the buffer is visible:
- `scroll_offset`: the buffer line index at the top of the viewport
- `scroll_sub_offset`: how many wrapped sub-rows to skip within that line
  (needed when a single long line wraps to more rows than the viewport height)
- `col_offset`: horizontal scroll in display columns (only used when soft-wrap
  is off)

**Viewport dimensions** — updated from the terminal size at the start of every
event-loop iteration:
- `height`: rows available for document content (terminal height minus the
  statusline row)
- `width`: total terminal columns

**Display config** — rarely-changing user preferences:
- `tab_width`, `soft_wrap`, `word_wrap`, `indent_wrap`
- `line_number_style` (Absolute / Relative / Hybrid)
- `gutter`: the `GutterConfig` describing which columns appear left of content
- `whitespace`: which whitespace characters get visual indicators

`ViewState` owns no buffer content and no cursor state. Those live on
`Document` and `SelectionSet` respectively, as siblings on `Editor`.

---

## Buffer lines vs visual lines

A **buffer line** is a sequence of chars ending in `\n`. There is a 1:1
mapping between buffer lines and rope lines.

A **visual row** (also called a display row or screen row) is what the user
sees as one row on screen. In the non-wrapping case, these are identical. With
soft-wrap enabled, a single long buffer line can produce several visual rows:
the first row plus one or more **continuation rows**.

This distinction matters everywhere:
- Line numbers count buffer lines, not visual rows — continuation rows show no
  number (or an optional wrap indicator character).
- The vertical scroll position is tracked in buffer lines (`scroll_offset`),
  with a sub-row component (`scroll_sub_offset`) for the fractional case.
- `ensure_cursor_visible` must count visual rows when soft-wrap is on, because
  a single long line may consume the entire viewport.

---

## DocumentFormatter: the single source of truth for row boundaries

`DocumentFormatter` (`src/ui/formatter.rs`) is a lazy iterator that yields one
`VisualRow` per visual row in the viewport. It is the **only place** in the
codebase that decides which buffer chars appear on which screen row.

Both the renderer and the cursor-position mapper consume `DocumentFormatter`.
Because they share the same source of truth, they can never disagree about
where a character is on screen — a common source of off-by-one bugs in editors
that compute these independently.

### What VisualRow carries

`VisualRow` is a small `Copy` struct with no references into the buffer:

- `row`: the 0-based visual row index from the top of the viewport
- `line_number`: the 1-based buffer line number (`None` for virtual rows with
  no buffer correspondence — e.g. future inline diagnostics)
- `is_continuation`: whether this is a soft-wrap continuation row
- `is_last_segment`: whether this is the last visual row of its buffer line
- `char_start` / `char_end`: the buffer char range this row displays
- `col_offset_in_line`: the display column of `char_start` relative to the
  line's first char (non-zero on continuation rows, used for tab alignment)
- `visual_indent`: indent padding columns to prepend on continuation rows when
  `indent_wrap` is enabled
- `trailing_ws_start`: the first char offset of trailing whitespace in this
  segment (used by the whitespace indicator renderer)

`VisualRow` being `Copy` and reference-free is what makes `DocumentFormatter`
a standard `Iterator` — no lending iterator lifetime gymnastics needed.

### How the formatter handles soft-wrap

When `soft_wrap` is off, each buffer line produces exactly one `VisualRow`.
No grapheme walk is needed — the formatter just emits segments based on line
indices.

When `soft_wrap` is on, the formatter walks graphemes across the line,
accumulating display columns via `grapheme_advance` (which handles tabs and
CJK double-width chars). When the column budget would be exceeded, it emits a
segment break. Three wrap policies stack on top of each other:

- **Character wrap** (always active when soft-wrap is on): break at any
  grapheme boundary.
- **Word wrap** (`word_wrap`): backtrack to the last word boundary before
  breaking, so words aren't split mid-word. Falls back to character wrap for
  words longer than the viewport.
- **Indent wrap** (`indent_wrap`): measure the buffer line's leading
  whitespace and set `visual_indent` on continuation rows, so they align with
  the content start of the first row.

The formatter starts at `scroll_offset`/`scroll_sub_offset` and stops after
`view.height` visual rows — it never scans the whole document.

### Performance

- Zero allocation per row: `VisualRow` is `Copy`.
- The internal `Vec<LineSegment>` that holds pre-computed segments for the
  current buffer line is reused across lines — at most one heap allocation
  ever, amortized O(1).
- O(viewport_height × avg_line_width) grapheme walks when wrapping; O(viewport_height)
  when not wrapping (no grapheme iteration needed at all).

---

## FrameContext: bounding what the renderer reads

Before the formatter loop, `render()` extracts a `FrameContext` from the
editor. This is a plain struct bundling the per-frame read-only data every row
needs: `buf`, `sels`, `mode`, `colors`, `highlights`, `col_offset`,
`tab_width`, `ws_cfg`.

`FrameContext` exists for two reasons:
1. It documents exactly which editor state the renderer depends on — no hidden
   reach into unrelated fields.
2. It makes `render_row_content` and `render_eol` independent of `&Editor`,
   which is the right step toward supporting split panes (where each pane
   renders a different document with shared theme/mode state).

---

## The renderer: styling and drawing

For each `VisualRow`, the renderer does two things: draws the gutter, then
draws the content.

### Style resolution

Per-character style follows a strict priority chain (highest wins):

1. **Cursor head** — the cell the primary cursor sits on (block cursor in Normal mode)
2. **Selection body** — any cell within a selection's inclusive range
3. **Highlights** — bracket match, search hits (via `HighlightMap`)
4. *(future slot)* — syntax highlighting
5. **Cursor line** — a subtle background tint on the primary cursor's entire line
6. **Default** — no decoration

This chain lives in `resolve_style()`. Adding a new style layer means inserting
one priority level in that function — nothing else changes.

### HighlightMap

`HighlightMap` (`src/ui/highlight.rs`) is a per-frame sorted list of
`(start, end, HighlightKind)` ranges. It is built once before the formatter
loop (in `Editor::update_highlight_cache()`, called from the event loop), then
queried per character via binary search during the render pass.

`HighlightKind` variants are ordered by ascending priority using derived `Ord`.
Currently: `SearchMatch < BracketMatch`. When syntax highlighting arrives, a
`Syntax` variant slots in at the bottom. Adding a new source means adding a
variant at the right position — no priority numbers, no if-else chains.

### Whitespace indicators

`WhitespaceConfig` controls three independent dimensions: spaces, tabs, and
newlines. Each can be `None` (never shown), `All` (always shown), or
`Trailing` (only trailing whitespace). The replacement characters (`·`, `→`,
`⏎`) are also configurable.

When a whitespace grapheme should be shown as an indicator, the renderer keeps
the base cell's background (so indicators inside selections show the selection
bg) but replaces the foreground with the whitespace color. The cursor head
cell is never patched — it must stay unambiguous.

### Gutter

The gutter is the strip of columns left of the content area. Its layout is
described by `GutterConfig` on `ViewState`: an ordered `Vec<GutterColumn>` and
an optional wrap indicator character.

`GutterColumn` is an enum. The only current variant is `LineNumber`, but the
design explicitly anticipates `Diagnostic`, `GitSigns`, and `FoldMarker`. Each
variant implements `width(total_lines)` and `render(...)`. Adding a new gutter
column is mechanical: add a variant, implement two methods, insert it into the
default config. No other code changes needed.

Line numbers support three styles:
- **Absolute** — every line shows its 1-based buffer number.
- **Relative** — every line shows its distance from the cursor; cursor line
  shows `0`.
- **Hybrid** (default) — cursor line shows absolute, others show relative.
  Best of both: you can jump by exact number and navigate by relative offset.

### Themes

All semantic colors live in `EditorColors` (`src/ui/theme.rs`) as ratatui
`Style` values: `default`, `cursor_head`, `selection`, `cursor_line`,
`bracket_match`, `search_match`, `whitespace`, `gutter`, `gutter_cursor_line`,
`tilde`, `statusline`, and per-mode status colors.

Producers (highlight map, selection logic) emit semantic kinds. The renderer
maps kinds to styles via `EditorColors`. This means changing a theme is a
single struct update — no rendering logic changes.

### Statusline

The statusline occupies the bottom row of the terminal. Its layout is
data-driven: `StatusLineConfig` holds left, center, and right sections, each
a `Vec<StatusElement>`. Elements are an enum: `Mode`, `FileName`, `Position`,
`Separator`, `SearchMatches`, `DirtyIndicator`, `MiniBuf`, etc.

The three-section layout degrades gracefully on narrow terminals: the center
section is dropped first, then the right. When a mini-buffer (command line,
search prompt) is active, the left section is replaced with the mini-buffer
input.

---

## How it all fits together: one frame

1. **Event loop** syncs `view.height`, `view.width`, `view.cached_total_lines`
   from the terminal.
2. `ensure_cursor_visible` updates `scroll_offset` and `scroll_sub_offset` so
   the cursor is on screen.
3. `update_highlight_cache` builds the `HighlightMap` for this frame (bracket
   match, search hits).
4. `render()` is called inside `term.draw()`:
   - Builds `FrameContext` from editor state.
   - Creates `DocumentFormatter`, iterates `VisualRow`s.
   - For each row: renders the gutter, then calls `render_row_content` which
     walks graphemes, resolves per-character styles, and writes cells.
   - Fills tilde rows past end-of-buffer.
   - Renders the statusline.
5. ratatui diffs the new `ScreenBuf` against the previous frame and emits only
   changed cells as escape codes.

---

## Futureproofing

The architecture anticipates several planned features without requiring
structural changes:

**Syntax highlighting** — `HighlightKind::Syntax` slots into the priority
chain below `SearchMatch`. A tree-sitter pass pushes ranges into the highlight
map before rendering. `resolve_style` gains one priority level.

**Inline diagnostics / ghost text** — `VisualRow.line_number` is
`Option<usize>`, already designed for rows with no buffer line. The formatter
would need an injection mechanism to insert virtual rows between buffer lines,
but the iterator contract and `VisualRow` shape require no changes.

**Git signs / diagnostic gutter** — add a `GutterColumn` variant. The width
and render methods implement the column's appearance. No other code changes.

**Split panes** — each pane gets its own `ViewState` and calls
`DocumentFormatter` independently. The main work is splitting `render()` to
accept a `RenderContext` (pane-local data) instead of `&Editor` (global).
`FrameContext` is already a step in this direction.
