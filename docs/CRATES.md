# hume-rope
### Depends on
- test-fixtures *(dev-only)*
### Used by
- hume-editing
- hume-engine
- hume-grid
- hume-treesitter
- hume-lsp
- hume-ops
- hume-editor
- hume-ui
- hume-decorations
- hume-scripting
- test-fixtures
## Description
Rope-domain primitives: line counting and ranges, grapheme-cluster boundaries and the typed cluster positions only it can mint (`ClusterStart`, `ClusterBound`, `ClusterRange`), buffer char offsets, display-column width, and LSP wire-position conversion. The single source of truth every other crate defers to for "how many lines" and "how wide is this text" — a pure math layer with no knowledge of buffers, selections, or rendering.

# hume-grid
### Depends on
- hume-rope
### Used by
- hume-platform
- hume-engine
- hume-editor
- hume-ui
## Description
The frame's cell grid: `Grid`/`Cell` storage, screen-region geometry, the style vocabulary the theme cascade composes, and the double-buffer diff that finds what's worth repainting between two frames. Pure data — no terminal, no I/O — so every invariant it enforces is testable without one; the half that talks to a terminal lives in `hume-platform`.

# hume-platform
### Depends on
- hume-grid
### Used by
- hume-lsp
- hume-scripting
- hume-editor
## Description
Platform abstraction layer: terminal control, frame presentation, process spawning, atomic file writes, and OS-specific directory conventions. Walls off every platform-specific code path so callers get one uniform, platform-independent signature.

# hume-editing
### Depends on
- hume-rope
- test-fixtures *(dev-only)*
### Used by
- hume-ops
- hume-lsp
- hume-treesitter
- test-fixtures
- hume-editor
- hume-decorations
- hume-scripting
## Description
Core text-editing model: the document (`BufferText`, a rope of Unicode scalar values with a recorded line-ending style), the selection model (`Selection`/`SelectionSet`, read and changed only paired with their text as `EditView`/`EditState`), edits as data (`EditBuilder` producing `ChangeSet`s, invertible and composable), and the undo tree (`History`). A pure data-and-algorithm layer — no knowledge of the editor, keymaps, rendering, or scripting.

# hume-engine
### Depends on
- hume-grid
- hume-rope
### Used by
- hume-scripting
- hume-treesitter
- hume-editor
- hume-ui
- hume-decorations
## Description
Rendering pipeline and pane geometry: the split/pane layout tree, the frame-render pipeline, decoration/statusline/tabline provider traits, and theming. Has no dependency on `hume-editing`: it renders from ropes and provider-supplied data, and paints selections the editor hands it as one `PaintedSelections` set with one primary, with no notion of edits or undo.

# hume-ops
### Depends on
- hume-editing
- hume-rope
- test-fixtures *(dev-only)*
### Used by
- hume-editor
- hume-scripting
## Description
Named commands — every edit and motion operation as a pure function of buffer + selections (plus command-specific params like `count` or `MotionMode`); edits also return a `ChangeSet`. Has no dependency on `hume-editor`, so "commands have no knowledge of keys" is compiler-enforced, not just discipline.

# hume-lsp
### Depends on
- hume-editing
- hume-platform
- hume-rope
### Used by
- hume-editor
## Description
LSP transport, JSON-RPC codec, client lifecycle state, and protocol-only wire decoding: locations, plus completion-item snippet-stripping/lenient-`TextEdit` decode helpers. Speaks protocol types only, plus opaque metadata the editor glue attaches — zero dependency on `Editor`, `Buffer`, or anything in `hume-editor`/`hume-engine`, so it stays independently testable.

# hume-scripting
### Depends on
- hume-editing
- hume-engine
- hume-ops
- hume-platform
- hume-rope
### Used by
- hume-editor
## Description
Steel (Scheme) scripting host: owns the Steel `Engine`, the plugin loading/activation pipeline, and the `EditorHost` capability-trait interface that builtins call into. Reaches editor state only through `EditorHost`, never a direct dependency on `hume-editor` — the inversion that keeps the workspace dependency graph acyclic despite scripting needing to drive almost everything else. `hume-editing`/`hume-ops` are the one exception: both are pure data-and-algorithm crates with no knowledge of `hume-editor` or live editor state, so reaching them directly for a stateless text transform (`split-words`, `hume-scripting/src/builtins/words.rs`) doesn't touch that inversion.

# hume-treesitter
### Depends on
- hume-editing
- hume-engine
- hume-rope
- test-fixtures *(dev-only)*
### Used by
- hume-editor
## Description
Tree-sitter integration: language/grammar registry, incremental parsing, syntax highlighting, and structural text-object queries. Knows only about buffers, ropes, and grammars — editor-domain glue (hooks, lazy-plugin activation, per-frame orchestration) stays in `hume-editor`.

# test-fixtures
### Depends on
- hume-editing
- hume-rope
### Used by
- hume-rope *(dev-only)*
- hume-editing *(dev-only)*
- hume-ops *(dev-only)*
- hume-treesitter *(dev-only)*
- hume-editor *(dev-only)*
## Description
Shared test infrastructure: a marker-annotated buffer/selection parsing DSL for editing-command tests, a corpus of Unicode text samples (`unicode`), plus grammar-fixture paths and gating for suites that need real tree-sitter grammars. Dev-dependency only; never part of a production build. `hume-rope` and `hume-editing` use it too, a dev-only cycle: their unit tests link a second copy of themselves through it, so they take only plain `&str` samples from it, never its DSL types.

# hume-ui
### Depends on
- hume-engine
- hume-grid
- hume-rope
### Used by
- hume-editor
## Description
The popup/menu/drawer/picker overlay widgets: geometry composition, shared box-drawing/scroll math, and `OverlayViews`, the single composition root for their shared view state. Every type here is a value object or a provider reading from a handle it was given — the raw overlay models live in `hume-editor` instead, decoration stores in the sibling `hume-decorations`.

# hume-decorations
### Depends on
- hume-editing
- hume-engine
- hume-rope
### Used by
- hume-editor
## Description
Steel-writable decoration stores (the write half `set-signs!`/`set-inlay-hints!`/etc. populates) and the concrete `hume_engine::providers` implementations reading from them (gutter signs, inlay hints, virtual lines, line backgrounds, highlights) — the write and read halves of every decoration kind, kept together. Never depends on `hume-editor` or constructs an `Editor`; the per-frame sync from store to provider handle stays in `hume-editor`'s `decoration_providers.rs` instead, since that reads live editor state directly.

# hume-editor
### Depends on
- hume-engine
- hume-grid
- hume-platform
- hume-scripting
- hume-editing
- hume-rope
- hume-ops
- hume-treesitter
- hume-lsp
- hume-ui
- hume-decorations
- test-fixtures *(dev-only)*
### Used by
- *(nothing — builds the `hume` binary)*
## Description
Editor state, scripting glue, keymaps, and the `hume` binary itself — the crate that ties every other crate together into a running editor. Owns `EditorState`, its `InputStack` (the stack of active input layers — modes and overlays alike — that decides which one handles a key, paste, or mouse event), the command dispatcher, keymap tries (Normal/Extend/Insert), the statusline, and the `EditorHost` implementation that `hume-scripting`'s builtins call into. UI widgets live in `hume-ui`, the decoration stores they render from in `hume-decorations`; `pane_state::build_pane` is the one place that wires both into a pane's `ProviderSet`.

# arch-lints
## Description
Architectural lints, enforced as `cargo test` integration tests that scan source files on disk for a pattern violating a rule — never by exercising the crate under scan's own code. Lives outside every product crate for exactly that reason: a lint that reads source as text has no business being *tests of* the crate it reads. One exception stays inside `hume-editor` instead (`editor/settings/manual_options_drift.rs`), since it calls that crate's own internals directly rather than scanning text.
