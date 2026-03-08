# HUME - Plan

## Tech Stack

| Component | Choice | Notes |
|-----------|--------|-------|
| Language | Rust | Memory-safe, expressive, excellent TUI ecosystem |
| Terminal I/O | TBD | crossterm is the likely choice (cross-platform) |
| TUI framework | TBD | ratatui, or raw crossterm for more control |
| Text storage | `ropey` | Rope-based buffer with structural sharing; enables tree-structured undo |
| Scripting | `steel` | Rust-native Scheme; plugins and configuration in the same language |
| Syntax highlighting | `tree-sitter` | Incremental parsing; also powers text objects and structural navigation |
| Build system | Cargo | Standard Rust tooling |

## Architecture (WIP)

To be designed. Key components will include:
- **Core**: Buffer management, text storage, edit operations
- **Editor**: Mode management, command handling, key mapping (keys → named commands, no key-to-key indirection)
- **Terminal**: Input handling, rendering, screen management
- **UI**: Status line, command bar, splits/tabs
- **Scripting**: Steel (Scheme) engine for plugins and configuration
- **LSP**: Rust transport/parsing layer + Steel scripts for behavior and customization

## Milestones

### M0 — Bootstrapping (current)
- [x] Project vision and README
- [x] Language decision: Rust
- [ ] Decide on core libraries (terminal I/O, TUI framework)
- [x] Decide on text storage data structure: rope via `ropey`
- [ ] Decide on editing model (vim-like, kakoune-like, hybrid)
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
