# HUME - Goals

## Vision

A modern, modal text editor that runs in the terminal. Built for the joy of building.

## Target Platforms

- **Primary**: macOS
- **Secondary**: Linux, Windows (Git Bash / WSL)
- **Terminal compatibility**: Modern terminals — prefer kitty keyboard protocol, fall back to legacy encoding when unavailable

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
| Scripting / Config | **Steel** (Rust-native Scheme) | Lisp syntax, designed for embedding in Rust. Helix is adopting it. Used for both plugins and configuration. |
| LSP architecture | **Hybrid** | Rust core handles transport and JSON-RPC parsing. Steel scripts handle behavior (diagnostics display, completion UX, keybindings). |
| Syntax highlighting | **Tree-sitter** | Incremental parsing, structural understanding. Enables text objects and structural navigation beyond just colors. Production-proven (Neovim, Helix, Zed, GitHub). |
| Key mapping | **Command-based** (Helix model) | Keys bind to named commands, not to other keys. No recursive/non-recursive distinction needed. Keymaps defined in Steel config. Supports nested keys for sequences/chords. |
| Editing model | **Select-then-act** (Helix/Kakoune) | Motions create selections, actions operate on them. No separate visual mode. Design for multiple selections from day one (`Vec<Selection>`). Text objects and keystroke macros supported. |
| Terminal I/O | **crossterm** | Cross-platform terminal I/O. Handles raw mode, key events, escape sequences. |
| Rendering | **ratatui as diffing engine** | Use ratatui's `Buffer`/`Terminal` for cell-level rendering and double-buffer diffing. No widgets. Immediate mode thinking with retained-mode optimization. |
| Terminal protocol | **Prefer kitty keyboard, fall back** | Detect kitty keyboard protocol support at startup. Use it when available for unambiguous key encoding, modifier reporting, and key release events. Fall back to legacy encoding otherwise (like Helix does). |

## Layer Responsibilities

| Layer | Responsibility |
|-------|---------------|
| **Core** | Buffer (rope), selections, edit operations, undo history. Knows nothing about keys or UI. |
| **Editor** | Modes, command dispatch, keymap lookup. Translates key events into core commands. |
| **Terminal/Renderer** | Screen output, display lines, overlays. Reads core state, never writes to it directly. |

## Modern Terminal Requirements

HUME requires **true color (24-bit)** and **synchronized output**. The **kitty keyboard protocol** is preferred but optional (graceful fallback).

### Terminal Capability Landscape

| Terminal | OS | True color | Kitty keyboard | Sync output | Notes |
|----------|-----|:---:|:---:|:---:|-------|
| **Kitty** | macOS, Linux | yes | yes | yes | Gold standard for protocol support |
| **Ghostty** | macOS, Linux | yes | yes | yes | New, very capable, fast adoption |
| **WezTerm** | macOS, Linux, Win | yes | yes | yes | Excellent cross-platform option |
| **Alacritty** | macOS, Linux, Win | yes | yes (0.15+) | yes | GPU-accelerated, minimal |
| **foot** | Linux (Wayland) | yes | yes | yes | Lightweight Wayland-native |
| **iTerm2** | macOS | yes | **no** (CSI u variant) | yes | Most popular macOS terminal — no kitty protocol |
| **Windows Terminal** | Windows | yes | partial | yes | Default Windows terminal |
| **Terminal.app** | macOS | yes (Ventura+) | no | no | Apple's built-in — limited |
| **GNOME Terminal** | Linux | yes | no | yes | Common Linux default |
| **Konsole** | Linux | yes | no | yes | KDE default |
