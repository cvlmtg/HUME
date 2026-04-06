# The Rendering Pipeline: Engine, Providers, and the 4-Stage Pipeline

Every frame, HUME takes a buffer full of text and produces a screen full of
styled terminal cells. This document explains the four-stage engine pipeline
that does that, the key data structures, and how the design accommodates future
features like syntax highlighting, diagnostics, and virtual lines.

## The four stages

```
Buffer (Rope) + Pane config
         │
         ▼
  Stage 1: Layout        ← "which buffer lines are visible, and how wide is the gutter?"
         │  → VisibleRange, gutter col_widths
         ▼
  Stage 2: Format        ← "where does each grapheme appear on screen?"
         │  → Vec<DisplayRow> + Vec<Grapheme> (per buffer line)
         ▼
  Stage 3: Style         ← "what colour/attribute does each grapheme get?"
         │  → Vec<ResolvedStyle> (one per Grapheme)
         ▼
  Stage 4: Compose       ← "write styled glyphs to the ratatui screen buffer"
         │
         ▼
  ratatui ScreenBuf      ← diffed against previous frame → terminal escape codes
```

The pipeline is **fused**: instead of materialising all rows for the full
visible range up front, `render_pane` processes one buffer line at a time
(`format → style → compose`). Peak scratch memory is O(graphemes/line) rather
than O(total visible graphemes).

## Key types

### `Pane` (`engine/src/pane.rs`)

Holds everything the engine needs to render one editor viewport:

- `buffer_id` — key into `EngineView::buffers` (rope lives in the editor's `Document`)
- `viewport: ViewportState` — `top_line`, `top_row_offset`, `horizontal_offset`, `width`, `height`
- `selections: Vec<Selection>` — all cursor/selection positions, pre-sorted by `head`
- `wrap_mode: WrapMode` — `None` or `Indent { width }`
- `providers: ProviderSet` — pluggable gutter columns, highlight sources, virtual lines, overlays

### `ProviderSet` (`engine/src/providers.rs`)

A collection of trait objects that inject content into the pipeline:

| Trait | Stage | Purpose |
|-------|-------|---------|
| `GutterColumn` | Compose | Line numbers, git signs, diagnostics icons |
| `HighlightSource` | Style | Syntax, bracket match, search matches |
| `VirtualLineSource` | Format | Inline diagnostics, git diff context |
| `InlineDecoration` | Format | Inline virtual text (e.g. type hints) |
| `OverlayProvider` | Compose | Floating overlays (completions, hover) |

Providers are registered by the editor at startup. The `SharedHighlighter` in
`editor/src/ui/highlight_providers.rs` wraps an `Arc<RwLock<Vec<...>>>` that
the editor writes once per frame, then the engine reads during Stage 3.

### `DisplayRow` and `Grapheme` (`engine/src/types.rs`)

Stage 2 (Format) produces one `DisplayRow` per visual row. A `DisplayRow` has:

- `kind: RowKind` — `LineStart { line_idx }`, `Continuation`, `VirtualLine`, or `Filler`
- `graphemes: Range<usize>` — index range into the shared `Vec<Grapheme>`
- `indent_depth` — for indent guides

Each `Grapheme` has:
- `text_range` — byte range within the line text
- `col` — visual column (accounting for tab width and CJK double-width)
- `width` — visual width in terminal columns
- `indent_depth`, `char_class`, `is_virtual`

### `ResolvedStyle` (`engine/src/types.rs`)

The output of Stage 3: one style per `Grapheme`. Combines foreground colour,
background colour, and modifier flags (bold, italic, underline, etc.) resolved
from the theme's scope hierarchy.

### `FrameScratch` (`engine/src/pipeline.rs`)

Reusable scratch storage cleared at the start of each pane render. All `Vec`s
stabilise their capacity after a few frames — no heap allocation in steady state.

## The fused loop (`render_pane` in `engine/src/pipeline.rs`)

```
pre: populate_sorted_sels, pre-collect virtual lines, compute col_widths
for each buffer line in visible range:
    drain_virtual_lines(Before)       ← virtual lines anchored above this line
    for each display row of this line:
        format_line_row(...)          ← Stage 2: graphemes + row geometry
        style_row(...)                ← Stage 3: highlight lookups, sel spans
        compose_row(...)              ← Stage 4: write to ratatui buffer
    drain_virtual_lines(After)        ← virtual lines anchored below this line
tilde filler rows until viewport is full
```

## Scope-based theming (`engine/src/theme.rs`, `engine/src/style.rs`)

Colours are resolved through a scope hierarchy (inspired by TextMate grammars).
A `ScopeId` is a small integer registered in `ScopeRegistry`. The `Theme`
maps scope IDs to `Style` values via prefix matching. Providers tag graphemes
with scope IDs; Stage 3 resolves them to `ResolvedStyle` by walking up the
scope tree until a match is found.

## Where the rope lives

The rope (`ropey::Rope`) never moves into the engine. `EngineView::render()`
accepts a closure `get_rope: impl Fn(BufferId) -> &Rope` so the editor can pass
a borrow of its `Document`'s rope without any cloning or ownership transfer.
This keeps the engine crate free of `Editor` dependencies.
