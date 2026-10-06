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
Rope-domain primitives: line and column counting, grapheme-cluster boundaries, typed buffer positions, display width, and LSP wire-position conversion.

# hume-grid
### Depends on
- hume-rope
### Used by
- hume-platform
- hume-engine
- hume-editor
- hume-ui
## Description
The frame's cell grid and the diff that finds what to repaint between two frames. Pure data, no terminal I/O.

# hume-platform
### Depends on
- hume-grid
### Used by
- hume-lsp
- hume-scripting
- hume-editor
## Description
Platform abstraction layer: terminal control, process spawning, atomic file writes, and OS directory conventions.

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
Core text-editing model: the document, selections, edits as data, and the undo tree.

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
Rendering pipeline and pane geometry: the layout tree, frame rendering, and theming.

# hume-ops
### Depends on
- hume-editing
- hume-rope
- test-fixtures *(dev-only)*
### Used by
- hume-editor
- hume-scripting
## Description
Named edit and motion commands, as pure functions of buffer and selections.

# hume-lsp
### Depends on
- hume-editing
- hume-platform
- hume-rope
### Used by
- hume-editor
- hume-scripting
## Description
LSP transport, JSON-RPC codec, client lifecycle state, and protocol-level wire decoding.

# hume-scripting
### Depends on
- hume-editing
- hume-engine
- hume-lsp
- hume-ops
- hume-platform
- hume-rope
### Used by
- hume-editor
## Description
Steel (Scheme) scripting host: runs plugins and configuration, and exposes editor functionality to them through a capability-trait interface.

# hume-treesitter
### Depends on
- hume-editing
- hume-engine
- hume-rope
- test-fixtures *(dev-only)*
### Used by
- hume-editor
## Description
Tree-sitter integration: grammar registry, incremental parsing, syntax highlighting, and structural text-object queries.

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
Shared test infrastructure: a marker-annotated buffer and selection DSL, Unicode text samples, and tree-sitter grammar fixtures. Dev-dependency only.

# hume-ui
### Depends on
- hume-engine
- hume-grid
- hume-rope
### Used by
- hume-editor
## Description
The popup, menu, drawer, and picker overlay widgets.

# hume-decorations
### Depends on
- hume-editing
- hume-engine
- hume-rope
### Used by
- hume-editor
## Description
Stores for the gutter signs, inlay hints, virtual lines, and highlights that scripts set, and the providers that render them.

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
Editor state, keymaps, command dispatch, and the `hume` binary: the crate that ties the others into a running editor.

# arch-lints
## Description
Architectural lints, run as `cargo test` integration tests that scan source files for rule violations.
