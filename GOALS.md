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
| Text storage | **Rope** (via `ropey` crate) | Efficient for large files, O(log n) edits anywhere, built-in line indexing. Undo uses changeset inversion; structural sharing provides cheap cloning at undo-tree branch points. Used by Helix. |
| Scripting / Config | **Steel** (Rust-native Scheme) | Lisp syntax, designed for embedding in Rust. Helix is adopting it. Used for both plugins and configuration. |
| LSP architecture | **Hybrid** | Rust core handles transport and JSON-RPC parsing. Steel scripts handle behavior (diagnostics display, completion UX, keybindings). |
| Syntax highlighting | **Tree-sitter** | Incremental parsing, structural understanding. Enables text objects and structural navigation beyond just colors. Production-proven (Neovim, Helix, Zed, GitHub). |
| Key mapping | **Command-based** (Helix model) | Keys bind to named commands, not to other keys. No recursive/non-recursive distinction needed. Keymaps defined in Steel config. Supports nested keys for sequences/chords. |
| Editing model | **Select-then-act** (Helix/Kakoune) | Motions create selections, actions operate on them. Design for multiple selections from day one (`Vec<Selection>`). Selections are always inclusive — `anchor == head` is a 1-char selection, never a zero-width point. Extend-selection variants are named commands orthogonal to motion type. Keybinding is an M3 concern. Text objects and keystroke macros supported. |
| Extend mode | **`x` toggle (primary) + Ctrl+motion (kitty bonus)** | A sticky mode where all motions extend the current selection instead of replacing it. Named "extend mode" (not "select mode") because motions already select in normal mode — what changes is that they extend. Bound to `x` (mnemonic: e**x**tend). Ctrl+motion was rejected as a universal modifier: fatal legacy-terminal collisions on 10 of 15 motion keys (`Ctrl+h`=Backspace, `Ctrl+j`=Enter, `Ctrl+[`=ESC, `Ctrl+b` eaten by tmux, etc.). Alt rejected — types accented chars on macOS, physical layout issues on Windows. **Bonus**: When kitty keyboard protocol is detected, `Ctrl+motion` also triggers extend as a chord shortcut without entering extend mode; gracefully absent in legacy terminals. |
| Delete char at cursor | **`d` (no separate binding needed)** | In select-then-act with always-inclusive 1-char selections, `d` (delete selection) on a fresh cursor deletes the char under it — identical to Vim's `x`. No dedicated binding required. `delete_char_forward` in `src/edit.rs` is for insert mode (the Delete key), not normal mode. |
| Terminal I/O | **crossterm** | Cross-platform terminal I/O. Handles raw mode, key events, escape sequences. |
| Rendering | **ratatui as diffing engine** | Use ratatui's `Buffer`/`Terminal` for cell-level rendering and double-buffer diffing. No widgets. Immediate mode thinking with retained-mode optimization. |
| Terminal protocol | **Prefer kitty keyboard, fall back** | Detect kitty keyboard protocol support at startup. Use it when available for unambiguous key encoding, modifier reporting, and key release events. Fall back to legacy encoding otherwise (like Helix does). |
| Documentation | **Markdown + auto-generated command reference** | Hand-written Markdown guides for concepts. Command reference auto-generated from Rust doc comments. In-editor `:help` renders Markdown in a read-only buffer. |
| Theming | **Hierarchical scopes** (Helix-compatible) | Dot-notation scopes (`keyword.function`, `ui.cursor`) with automatic fallback. Follow Helix scope convention. Read Helix TOML themes natively; Steel themes as primary format. Discoverability via `:theme-debug` and token inspection. |
| Package manager | **Built-in, declarative, replaceable** | Config declares plugins (`username/repo`). `:plugin-sync` reconciles disk to config (install/update/remove). Git-based, no registry, no auto dependency resolution. Built in Steel, replaceable by users. |
| Indent queries | **Helix format** (`indent.scm`) | Reuse Helix's existing per-language indent queries directly. No drawbacks identified; avoids reinventing a query format and gives us a large library of languages for free. |
| Unicode handling | **Grapheme clusters from day one** | All motions and selections operate on grapheme clusters via `unicode-segmentation`, not bytes or chars. Handles emoji, combining characters, CJK wide chars correctly. Avoids painful retrofitting. |
| Symbol rename | **LSP-first, tree-sitter fallback** | Use `textDocument/rename` when an LSP server is active — it is scope-aware and works across files. Fall back automatically to a tree-sitter local rename (using `locals.scm` scope queries) when no LSP is available — file-local only, but still scope-correct within the file. Same keybinding in both cases; the degraded fallback is transparent to the user. |

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
| **tmux** | cross-platform | yes (with config) | **no** | yes (3.3+) | Multiplexer — sits between emulator and app; does not pass through kitty protocol |

## Open Questions

| Question | Context |
|----------|---------|
| Multiline quote text objects | Quote text objects (`i"`, `i'`, `` i` ``) are line-bounded because the parity scan gives wrong results when earlier lines contain unmatched quotes. Brackets don't have this problem (asymmetric delimiters allow depth tracking). Tree-sitter can resolve the ambiguity — use syntax-aware matching when a grammar is loaded, fall back to line-bounded parity otherwise. |
