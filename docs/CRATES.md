# hume-rope
### Used by
- hume-editing
- hume-engine
- hume-treesitter
- hume-lsp
- hume-editor
## Description
Rope-domain primitives for line counts, line ranges, grapheme boundaries, and rope-position math, shared by every crate that needs to answer "how many lines" or "which line is last." Distinguishes *ropey domain* (ropey's own line indexing, including the phantom trailing line the buffer invariant's structural `\n` creates) from *content domain* (that phantom line excluded); its six functions are the single source of truth for line-count/range math workspace-wide. Its `width` module (`tab_advance`, `grapheme_width`, `str_width`) is likewise the single source of truth for display-column math — the only crate visible to both `hume-editing` (editing-ops tab math) and `hume-engine` (the renderer), so it's where the two converge on one convention.

# hume-platform
### Used by
- hume-engine
- hume-lsp
- hume-scripting
- hume-editor
## Description
Platform abstraction layer — terminal control (raw-mode lifecycle, kitty keyboard protocol, synchronized output, via `termina`), process spawning with process-group/reap discipline, atomic file writes, and OS-specific config/data/runtime directory conventions. All `#[cfg(unix)]`/`#[cfg(windows)]` code is walled off inside this crate so every caller gets a uniform, platform-independent signature.

# hume-editing
### Depends on
- hume-rope
### Used by
- hume-ops
- hume-lsp
- hume-treesitter
- hume-test-fixtures
- hume-editor
## Description
Core text-editing model: the document (`Text`, a rope of Unicode scalar values with a recorded line-ending style), the cursor model (`Selection`/`SelectionSet`), edits as data (`ChangeSet`, invertible and composable), and the undo tree (`History`), plus grapheme-cluster boundary utilities. A pure data-and-algorithm layer — no knowledge of the editor, keymaps, rendering, or scripting.

# hume-engine
### Depends on
- hume-platform
- hume-rope
### Used by
- hume-scripting
- hume-treesitter
- hume-editor
## Description
Rendering pipeline and pane geometry — the layout tree (splits and panes), the frame-render pipeline that turns rope content plus provider data into ratatui cells, decoration/statusline/tabline provider traits, and theming. Deliberately has no dependency on `hume-editing`: it renders from ropes and provider-supplied data and has no notion of selections, edits, or undo.

# hume-ops
### Depends on
- hume-editing
- hume-test-fixtures *(dev-only)*
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
LSP transport, JSON-RPC codec, and client lifecycle state. Speaks `BufferId`-free protocol types (`lsp_types`) plus opaque metadata the editor glue attaches; zero dependency on `Editor`, `Buffer`, or anything in `hume-editor`/`hume-engine`, following the `hume-treesitter` precedent so the crate stays acyclic and independently testable.

# hume-scripting
### Depends on
- hume-engine
- hume-platform
### Used by
- hume-editor
## Description
Steel (Scheme) scripting host — owns the Steel `Engine`, the plugin loading/activation pipeline, and the `EditorHost` capability-trait interface that builtins call into. Runs entirely on the main event-loop thread, since Steel's `Engine` is `!Send`. Reaches editor state through `EditorHost` rather than depending on `hume-editor` directly; that inversion is what keeps the workspace dependency graph acyclic despite scripting needing to drive almost everything else.

# hume-treesitter
### Depends on
- hume-editing
- hume-engine
- hume-rope
- hume-test-fixtures *(dev-only)*
### Used by
- hume-editor
## Description
Tree-sitter integration: language/grammar registry with dynamic loading, the background incremental-parse worker, syntax highlighting, and embedded-language injection resolution. Editor-domain glue (hooks, lazy-plugin activation, the per-frame orchestration that ties this crate's parse backend to a live `Editor`) stays in `hume-editor`; this crate only knows about buffers, ropes, and grammars.

# hume-test-fixtures
### Depends on
- hume-editing
### Used by
- hume-ops *(dev-only)*
- hume-treesitter *(dev-only)*
- hume-editor *(dev-only)*
## Description
Shared test infrastructure — the marker-annotated buffer/selection parsing DSL (`parse_state`/`serialize_state`/`assert_state!`) used by editing-command tests, plus grammar-fixture paths and require-fixtures gating shared by test suites that need real tree-sitter grammars. Dev-dependency only; never part of a production build.

# hume-editor
### Depends on
- hume-engine
- hume-platform
- hume-scripting
- hume-editing
- hume-rope
- hume-ops
- hume-treesitter
- hume-lsp
- hume-test-fixtures *(dev-only)*
### Used by
- *(nothing — builds the `hume` binary)*
## Description
Editor state, scripting glue, keymaps, UI widgets, and the `hume` binary itself — the crate that ties every other crate together into a running editor. Owns `EditorState`, the command dispatcher, keymap tries (Normal/Extend/Insert), minibuffer/completion/picker UI, and the `EditorHost` implementation that `hume-scripting`'s builtins call into.
