# HUME - Plan

## Tech Stack

| Component | Choice | Notes |
|-----------|--------|-------|
| Language | Rust | Memory-safe, expressive, excellent TUI ecosystem |
| Terminal I/O | `crossterm` | Cross-platform terminal I/O; kitty keyboard protocol preferred with legacy fallback |
| Rendering | `ratatui` (diffing only) | Cell-level Buffer/Terminal for double-buffer diffing; no widgets |
| Text storage | `ropey` | Rope-based buffer with structural sharing; enables tree-structured undo |
| Scripting | `steel` | Rust-native Scheme; plugins and configuration in the same language |
| Syntax highlighting | `tree-sitter` | Incremental parsing; also powers text objects and structural navigation |
| Build system | Cargo | Standard Rust tooling |

## Architecture (WIP)

To be designed. Key components will include:
- **Core**: Buffer management, text storage, edit operations, selections (`Vec<Selection>` from day one)
- **Editor**: Mode management, command handling, key mapping (keys → named commands, no key-to-key indirection)
- **Terminal**: Input handling, rendering, screen management. **Important**: The renderer must iterate over "display lines" (not buffer lines) from day one. A display line is either a real buffer line or a virtual line. Initially every display line maps 1:1 to a buffer line, but this abstraction is required for virtual lines later and is expensive to retrofit.
- **Layout**: Custom layout system — divides screen `Rect` into sub-regions (tab bar, editor panes, status line, command line). Splits are nested `Rect` divisions.
- **Overlays**: Completion menus, popups, hover info — rendered last on top of main content. Ratatui diffs handle cleanup on dismiss.
- **UI**: Tab bar, status line, command line, split panes
- **Decorations**: Annotation layer for virtual lines/text (diagnostics, ghost text, code lenses, inlay hints, git blame). Buffer-position-anchored, auto-updated on edits, queryable by line. Multiple sources (LSP, plugins, git).
- **Scripting**: Steel (Scheme) engine for plugins and configuration
- **LSP**: Rust transport/parsing layer + Steel scripts for behavior and customization

## Milestones

### M0 — Bootstrapping (current)
- [x] Project vision and README
- [x] Language decision: Rust
- [x] Decide on core libraries: crossterm + ratatui (diffing engine only)
- [x] Decide on text storage data structure: rope via `ropey`
- [x] Decide on editing model: Helix-style select-then-act
- [ ] Initialize Rust project with Cargo
- [ ] First render: open a file and display it

### M1 — Minimal viewer
- [ ] Open and display a file with scrolling
- [ ] Line numbers
- [ ] Status bar with filename and position
- [ ] Quit command

### M2 — Modal editing
- [ ] Normal mode with cursor movement
- [ ] Insert mode with text input
- [ ] Command mode (`:` commands)
- [ ] Basic editing: insert, delete, backspace

### Future milestones
- Tree-structured undo/redo (vim-style undo tree)
- Search and replace
- Multiple buffers / splits
- Syntax highlighting via tree-sitter
- Steel scripting engine + plugin API
- Configuration via Steel
- LSP support (Rust transport + Steel behavior layer)
- Virtual lines / decoration layer (inline diagnostics, ghost text, code lenses, inlay hints)
