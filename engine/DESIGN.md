> **STATUS: IMPLEMENTED** — The engine described here shipped as part of the M7 multi-buffer migration (complete 2026-04-20). This document is preserved as a historical requirements brief.

## GOAL

Design a rendering engine for a modal text editor that is elegant, fast, and composable.

The core pipeline should be minimal — it orchestrates stages, owns no feature logic. Every feature (gutter columns, highlights, virtual lines, overlays) plugs into the pipeline through well-defined extension points. A new feature (e.g. git gutter signs) should be addable without modifying existing pipeline code — only by registering a new provider.

Features belong in the core only when extracting them would force the pipeline to expose internal state, add an indirection hop in the per-grapheme hot path, or break the single-source-of-truth for position mapping.

## ROLE OF THIS DOCUMENT

This document is the **requirements input** for the design phase. It defines goals, features, performance constraints, and coding principles. It does not prescribe architecture — that is the design phase's job.

## DESIGN PHASE

Run two parallel agents. One designs the engine (pipeline stages, data structures, extension points, data flow). The other challenges every decision (simpler alternative? hidden coupling? allocation? SSOT violation?). The design phase continues until both agents converge on a final architecture, or it becomes clear they have reached an irresolvable disagreement (flag it for the user).

## ARCHITECTURAL CONSTRAINTS

These are day-one requirements, not future additions. The engine must be designed to accommodate them from the first line of code — retrofitting is expensive.

- **Clean-sheet design**: the existing codebase is not a constraint. Design the best possible engine from scratch — existing code will be adapted to the new engine, not the other way around. Do not preserve compatibility with current types, function signatures, or data flow. The design agents must ignore the current implementation entirely.
- **Language**: the engine is written in Rust, 2024 edition. All design decisions must be idiomatic Rust — ownership, borrowing, and lifetime discipline are not afterthoughts. The architecture must be expressible without unsafe.
- **Rope-backed buffer**: the editor uses the `ropey` crate as its rope data structure. The engine receives text via `ropey`'s API (`Rope`, `RopeSlice`, chunk iterators). All text access — grapheme walks, byte-to-char conversions, line indexing — must go through `ropey`'s interfaces. The engine must never assume a contiguous `&str` over the full buffer; it must be designed to work with chunked iteration from day one.
- **Syntax highlighting via tree-sitter**: syntax highlighting is powered by tree-sitter. The engine consumes tree-sitter's incremental parse trees and highlight queries — it does not implement its own parser or regex-based highlighter. The design must account for tree-sitter's chunk-based `Input` API (which aligns naturally with `ropey` chunk iterators) and for incremental re-highlighting on edit (only the edited subtree is re-parsed).
- **Terminal output via ratatui**: the engine writes into ratatui's `Buffer` (a flat grid of `Cell`s). This is the engine's output boundary — all style resolution must ultimately produce values expressible as ratatui's `Color` and `Modifier` types. The engine's internal style representation may be richer, but the final cell-write step is a mapping into ratatui's model. The engine must not bypass ratatui to write directly to the terminal.
- **Split panes**: the engine must support multiple independent viewports (panes) from the start. Each pane owns its scroll state, cursor, and gutter config. The engine doesn't need to *implement* a split UI immediately, but its data model must make adding panes trivial — no global state that assumes a single content area.
- **Composable pipeline**: the render pipeline is a sequence of stages. Each stage does one thing. New features slot in as providers/consumers at defined extension points, not as modifications to existing stages.
- **Provider model for extensible features**: gutter columns, highlight sources, virtual line sources, and overlays are all registered providers. Adding a new git-signs column or a new diagnostic highlight source means adding a provider — not modifying the pipeline or existing providers.

## FEATURES

The engine is designed from the ground up to support all of the following. This is the full feature set — the architecture must accommodate all of them:

### Layout
- Split panes and multiple windows
- Per-pane regions: gutter, content area, optional right-side column (scrollbar, minimap)
- Statusline (bottom row), optional tab bar
- Tilde rows past end of buffer

### Viewport & Scrolling
- Vertical scroll with configurable look-ahead margin
- Horizontal scroll with look-ahead margin (disabled when soft-wrap is on)
- Soft wrap (long lines wrap to next display row instead of scrolling)
- Word wrap (wrap breaks prefer whitespace boundaries)
- Indent wrap (continuation rows indented to the buffer line's indent level)
- Sub-row scroll offset for buffer lines taller than the viewport

### Gutter
- Line numbers: absolute, relative, or hybrid (default) styles
- Dynamic gutter width that grows with line count
- Continuation row gutter: blank or optional wrap indicator character
- Composable column architecture (variants: LineNumber; slots for diagnostics, git signs, fold markers, etc)

### Character Rendering
- Grapheme-cluster-aware rendering (no raw char stepping)
- CJK double-width character support
- Tab expansion to configurable tab stops, with correct alignment across wrapped segments
- Whitespace indicators: spaces, tabs, newlines — each independently set to None, All, or Trailing
- Customizable whitespace indicator characters (·, →, ⏎)

### Style Resolution (per-character priority cascade)

Priority is highest-first. Within a tier, the highest-priority provider wins.

1. Cursor head cell
2. Selection range
3. Highlights (in descending priority within this tier):
   - Bracket match (highest — must remain visible even inside a search match)
   - Diagnostic highlighting
   - Search match
   - Syntax highlighting (lowest — broad coverage, easily overridden)
4. Cursor line background tint
5. Default

### Multi-cursor rendering
- All selections rendered simultaneously — the engine iterates `Vec<Selection>`, not a single cursor
- Each selection has a head cell and an inclusive range; both are styled independently
- Cursor-line highlight: when multiple cursors are active, every cursor line receives the highlight (not just the primary)

### Cursor
- Mode-aware cursor shape: block (Normal/Extend), bar (Insert/Command/Search/Select)
- Cursor hidden in Normal mode (visual block cell acts as cursor)
- Bar cursor tracked to exact grapheme position in Insert mode

### Indent guides
- Vertical lines drawn in the content area at each indent level
- Computed from the formatter's indent-depth information — no separate pass
- Style-resolved independently from character highlighting (do not interfere with syntax or selection highlights)

### Text decorations
- The style system supports modifiers beyond fg/bg color: underline (solid, wavy, dotted, dashed), strikethrough, bold, italic, undercurl
- Decorations compose with highlights — a diagnostic wavy underline overlays syntax color without replacing it
- The style resolution cascade produces a composite style (color + modifiers), not just a color

### Theme
- A theme resolver maps semantic scope names (e.g. `keyword.function`, `ui.cursor.match`) to concrete styles
- Scopes use dot-notation with automatic fallback: `keyword.function` falls back to `keyword`, then to default
- All highlight providers emit scope names, not raw styles — the theme resolver is the single source of style

### Virtual lines / decoration layer

Virtual lines are display rows with no corresponding buffer line. They are injected into the formatter's output stream between real buffer lines (or within them, for inline decorations). The architecture must define a clean injection point — virtual line producers should not need to know about wrapping, tab expansion, or gutter rendering.

Types of virtual content to support:
- Inline diagnostics (below the offending line)
- Ghost text (inline, rendered at a buffer position, non-editable)
- Code lenses (above a definition)
- Inlay hints (inline type/parameter annotations)
- Git blame (end-of-line or virtual line)

### Overlays

Overlays are rendered on top of the content area, at a specific anchor position. They are independent of the main pipeline — the pipeline renders first, overlays composite last.

Types of overlays to support:
- Completion menu
- Hover info / documentation popup
- Diagnostic detail popup
- Register picker, file picker, any future picker UI

## PERFORMANCE

Performance comes from choosing correct algorithms and data structures. The hot paths are: the per-frame formatter loop, the per-row renderer loop, and the per-grapheme style resolution. These must be allocation-free.

Memory allocations:
- No heap allocations inside the per-frame formatter loop, the per-row renderer loop, or per-grapheme style resolution
- Allocate once per frame (or less), reuse within the frame via scratch buffers passed by `&mut`
- Allocate once at startup for structures that don't change (provider lists, config)
- Constant strings: use `&'static str` or `Cow::Borrowed` — no heap allocation for string literals

Calculations:
- Do not recompute a value in the same frame if it was already computed — especially if computing it involves allocation or a grapheme walk
- Trivial arithmetic may be repeated freely — prefer clarity over micro-caching for cheap operations

## CORE RULES

- **Separation of concerns**: every pipeline stage does exactly one thing. The formatter knows nothing about screen cells. The renderer knows nothing about wrap logic. The gutter knows nothing about content. Violations are bugs.
- **Single Source of Truth**: a value is computed in one place and read everywhere else. No duplicated state. If two pieces of code need the same derived value, one computes it and passes it — they do not each compute it independently.
- **Don't Repeat Yourself**: if the same logic appears in two places, extract a helper. The threshold is roughly 3+ lines of non-trivial code. Single-expression helpers are over-engineering — inline them.
