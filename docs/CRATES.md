# hume-rope
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
- test-fixtures *(dev-only)*
## Description
Rope-domain primitives: line counting and ranges, grapheme-cluster boundaries, buffer char offsets, display-column width, and LSP wire-position conversion. The single source of truth every other crate defers to for "how many lines" and "how wide is this text" — a pure math layer with no knowledge of buffers, selections, or rendering.

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
### Used by
- hume-ops
- hume-lsp
- hume-treesitter
- test-fixtures
- hume-editor
- hume-decorations
## Description
Core text-editing model: the document (`BufferText`, a rope of Unicode scalar values with a recorded line-ending style), the cursor model (`Selection`/`SelectionSet`), edits as data (`ChangeSet`, invertible and composable), and the undo tree (`History`), plus grapheme-cluster boundary utilities. A pure data-and-algorithm layer — no knowledge of the editor, keymaps, rendering, or scripting.

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
Rendering pipeline and pane geometry: the split/pane layout tree, the frame-render pipeline, decoration/statusline/tabline provider traits, and theming. Deliberately has no dependency on `hume-editing`: it renders from ropes and provider-supplied data and has no notion of selections, edits, or undo. Also carries `lock::LockExt`, the shared `RwLock`-poisoning policy for the frame-local `Arc<RwLock<_>>` state `hume-ui` and `hume-decorations` each hand the render pipeline — a leaf-crate utility, not a rendering concern of its own, but both of that state's owners already depend on this crate.

# hume-ops
### Depends on
- hume-editing
- hume-rope
- test-fixtures *(dev-only)*
### Used by
- hume-editor
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
LSP transport, JSON-RPC codec, client lifecycle state, and protocol-only wire decoding (locations, completion items). Speaks protocol types only, plus opaque metadata the editor glue attaches — zero dependency on `Editor`, `Buffer`, or anything in `hume-editor`/`hume-engine`, so it stays independently testable.

# hume-scripting
### Depends on
- hume-engine
- hume-platform
- hume-rope
### Used by
- hume-editor
## Description
Steel (Scheme) scripting host: owns the Steel `Engine`, the plugin loading/activation pipeline, and the `EditorHost` capability-trait interface that builtins call into. Reaches editor state only through `EditorHost`, never a direct dependency on `hume-editor` — the inversion that keeps the workspace dependency graph acyclic despite scripting needing to drive almost everything else.

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
- hume-ops *(dev-only)*
- hume-treesitter *(dev-only)*
- hume-editor *(dev-only)*
## Description
Shared test infrastructure: a marker-annotated buffer/selection parsing DSL for editing-command tests, plus grammar-fixture paths and gating for suites that need real tree-sitter grammars. Dev-dependency only; never part of a production build.

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
Editor state, scripting glue, keymaps, and the `hume` binary itself — the crate that ties every other crate together into a running editor. Owns `EditorState`, the command dispatcher, keymap tries (Normal/Extend/Insert), the statusline, and the `EditorHost` implementation that `hume-scripting`'s builtins call into. UI widgets live in `hume-ui`, the decoration stores they render from in `hume-decorations`; `pane_state::build_pane` is the one place that wires both into a pane's `ProviderSet`.

# arch-lints
## Description
Architectural lints, enforced as `cargo test` integration tests that scan source files on disk for a pattern violating a rule — never by exercising the crate under scan's own code. Lives outside every product crate for exactly that reason: a lint that reads source as text has no business being *tests of* the crate it reads. One exception stays inside `hume-editor` instead (`editor/settings/manual_options_drift.rs`), since it calls that crate's own internals directly rather than scanning text.
