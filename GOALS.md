# HUME - Goals

## Vision

A modern, modal text editor that runs in the terminal. Built for the joy of building.

## Target Platforms

- **Primary**: macOS
- **Secondary**: Linux, Windows (Git Bash / WSL)
- **Terminal compatibility**: Modern terminals only — no legacy protocol support

## Design Principles

- Modal editing (inspired by vim/kakoune/helix)
- Modern terminal capabilities (24-bit color, kitty keyboard protocol, etc.)
- Clean, maintainable codebase over feature completeness
- This is a learning playground — correctness and elegance over shipping deadlines

## Decisions Made

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | **Rust** | Memory-safe, zero-cost abstractions, best TUI ecosystem (crossterm + ratatui). Expressive type system with pattern matching, algebraic types, and macros. Ideal for a learning project. |
| Text storage | **Rope** (via `ropey` crate) | Efficient for large files, O(log n) edits anywhere, built-in line indexing. Structural sharing makes tree-structured undo cheap. Used by Helix. |

## Open Questions

- Editing model: vim-like vs kakoune-like (select-then-act) vs something new?
- Rendering approach: immediate mode vs retained mode?
- Configuration format: TOML, Lua scripting, or something else?
- Syntax highlighting: tree-sitter integration?
- LSP support: yes/no, and when?
