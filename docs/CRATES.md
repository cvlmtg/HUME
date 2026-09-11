# hume-rope
### Used by
- hume-editing
- hume-engine
- hume-grid
- hume-treesitter
- hume-lsp
- hume-ops
- hume-editor
- hume-scripting
- test-fixtures *(dev-only)*
## Description
Rope-domain primitives for line counts, line ranges, grapheme boundaries, buffer char offsets, and rope-position math, shared by every crate that needs to answer "how many lines" or "which line is last." Distinguishes *ropey domain* (ropey's own line indexing, including the phantom trailing line the buffer invariant's structural `\n` creates) from *content domain* (that phantom line excluded) — its `line` module wraps the two in distinct types (`RopeyLine`/`ContentLine`/`RopeyLineCount`/`ContentLineCount`) so the compiler rejects mixing them, and its `lines` module's four line-count/range functions (built on those types) are the single source of truth for that math workspace-wide. Its `grapheme` module is the sole grapheme-cluster boundary walker (`next_grapheme_boundary`/`prev_grapheme_boundary`/`snap_to_cluster_start`) every motion and selection operation goes through, and its `cursor` module is the codepoint-level `CharCursor` scanner those boundary walkers (and ASCII-delimiter scans elsewhere) are built on. Its `position_encoding` module converts LSP wire positions (UTF-16 code units or bytes, by negotiated encoding) to and from `CharOffset`/line indices — the one place those wire units cross into rope-domain math. Its `offset` module wraps every buffer char position in `CharOffset` (private field, no arithmetic trait impls — a raw `+ 1`/`- 1` on a buffer position doesn't type-check) and names the two range conventions the codebase mixes as `ExclusiveRange<CharOffset>`/`InclusiveRange<CharOffset>`, the domain type behind the "Buffer char offsets" invariant in `CLAUDE.md`. Its `width` module (`tab_advance`, `grapheme_width`, `str_width`) is likewise the single source of truth for display-column math — depended on directly by every crate that measures it: `hume-ops` (editing-ops tab math), `hume-engine` (the renderer), `hume-grid` (`Canvas`'s frame-writing primitives), `hume-editor` (UI chrome), and `hume-scripting` (the `:plugin-status` table), so all five converge on one convention. Deliberately stays primitive `usize`/`u32` rather than domain-typed, since it's called with display-line-relative, buffer-line-relative, and terminal-cell-relative columns alike. Its `column` module is where those origins *are* domain-typed: `DisplayLineCol`/`BufferLineCol` (terminal cells, split by whether they're measured from a display line or a buffer line — the two coincide only without wrap), `CharCol`, `GraphemeCol`, and `ByteCol`, the domain types behind the "Line/buffer columns" invariant in `CLAUDE.md`.

# hume-grid
### Depends on
- hume-rope
### Used by
- hume-platform
- hume-engine
- hume-editor
## Description
The frame's cell grid: the `Cell`/`Grid` storage HUME draws into, the `Rect`/`Position` geometry that describes screen regions, the `ResolvedStyle`/`Rgb` style vocabulary the theme cascade composes, the double-buffer diff that turns two consecutive frames into the cells worth repainting, and `Canvas` — the frame's single text writer, the only thing built directly on `Grid`'s own cell-mutating primitives (`set_glyph`/`fill_span`, `pub(crate)` for exactly that reason). Pure data plus that one text-measuring dependency (`hume-rope`, for `Canvas`'s width model) — no terminal, no I/O, no other HUME crate — so every invariant it enforces is testable without a terminal; the half that talks to a terminal lives in `hume-platform`. A cell stores the display width its writer measured rather than re-deriving it, which is what keeps the diff and the emitter agreeing with `hume-rope`'s width model instead of carrying a second one — the same model `Canvas` measures with, since `hume-rope` is a real dependency of this crate.

# hume-platform
### Depends on
- hume-grid
### Used by
- hume-lsp
- hume-scripting
- hume-editor
## Description
Platform abstraction layer — terminal control (raw-mode lifecycle, kitty keyboard protocol, synchronized output, via `termina`), frame presentation (the double-buffered `Screen`: front/back `hume-grid` grids, the gap policy the diff runs under, and the escape-sequence emitter that carries a composed frame to the terminal), process spawning with process-group/reap discipline, atomic file writes, and OS-specific config/data/runtime directory conventions. All `#[cfg(unix)]`/`#[cfg(windows)]` code is walled off inside this crate so every caller gets a uniform, platform-independent signature.

# hume-editing
### Depends on
- hume-rope
### Used by
- hume-ops
- hume-lsp
- hume-treesitter
- test-fixtures
- hume-editor
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
## Description
Rendering pipeline and pane geometry — the layout tree (splits and panes), the frame-render pipeline that turns rope content plus provider data into `hume-grid` cells, decoration/statusline/tabline provider traits, and theming. Deliberately has no dependency on `hume-editing`: it renders from ropes and provider-supplied data and has no notion of selections, edits, or undo.

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
LSP transport, JSON-RPC codec, and client lifecycle state. Speaks `BufferId`-free protocol types (`lsp_types`) plus opaque metadata the editor glue attaches; zero dependency on `Editor`, `Buffer`, or anything in `hume-editor`/`hume-engine`, following the `hume-treesitter` precedent so the crate stays acyclic and independently testable.

# hume-scripting
### Depends on
- hume-engine
- hume-platform
- hume-rope
### Used by
- hume-editor
## Description
Steel (Scheme) scripting host — owns the Steel `Engine`, the plugin loading/activation pipeline, and the `EditorHost` capability-trait interface that builtins call into. Runs entirely on the main event-loop thread, since Steel's `Engine` is `!Send`. Reaches editor state through `EditorHost` rather than depending on `hume-editor` directly; that inversion is what keeps the workspace dependency graph acyclic despite scripting needing to drive almost everything else.

# hume-treesitter
### Depends on
- hume-editing
- hume-engine
- hume-rope
- test-fixtures *(dev-only)*
### Used by
- hume-editor
## Description
Tree-sitter integration: language/grammar registry with dynamic loading, the background incremental-parse worker, syntax highlighting, embedded-language injection resolution, and structural text-object/navigation queries (function/class/argument/comment/unit-test/value spans). Editor-domain glue (hooks, lazy-plugin activation, the per-frame orchestration that ties this crate's parse backend to a live `Editor`) stays in `hume-editor`; this crate only knows about buffers, ropes, and grammars.

# test-fixtures
### Depends on
- hume-editing
- hume-rope
### Used by
- hume-ops *(dev-only)*
- hume-treesitter *(dev-only)*
- hume-editor *(dev-only)*
## Description
Shared test infrastructure — the marker-annotated buffer/selection parsing DSL (`parse_state`/`serialize_state`/`assert_state!`) used by editing-command tests, plus grammar-fixture paths and require-fixtures gating shared by test suites that need real tree-sitter grammars. Dev-dependency only; never part of a production build.

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
- test-fixtures *(dev-only)*
### Used by
- *(nothing — builds the `hume` binary)*
## Description
Editor state, scripting glue, keymaps, UI widgets, and the `hume` binary itself — the crate that ties every other crate together into a running editor. Owns `EditorState`, the command dispatcher, keymap tries (Normal/Extend/Insert), minibuffer/completion/picker UI, and the `EditorHost` implementation that `hume-scripting`'s builtins call into.
