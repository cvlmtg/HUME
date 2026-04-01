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
| Extend mode | **`e` toggle (primary) + Ctrl+motion (kitty bonus, deferred to M4)** | A sticky mode where all motions extend the current selection instead of replacing it. Named "extend mode" (not "select mode") because motions already select in normal mode — what changes is that they extend. Bound to `e` (mnemonic: **e**xtend). `x` was previously considered but repurposed for line selection. Ctrl+motion was rejected as a universal modifier: fatal legacy-terminal collisions on 10 of 15 motion keys (`Ctrl+h`=Backspace, `Ctrl+j`=Enter, `Ctrl+[`=ESC, `Ctrl+b` eaten by tmux, etc.). Alt rejected — types accented chars on macOS, physical layout issues on Windows. **Bonus** (deferred to M4): When kitty keyboard protocol is detected, `Ctrl+motion` also triggers extend as a chord shortcut without entering extend mode; gracefully absent in legacy terminals. |
| Line selection | **`x`/`X` (walk down/up lines)** | `x` selects the full current line including `\n`; repeated `x` walks to the next line. `X` does the same backward. In extend mode (or `Ctrl+x`/`Ctrl+X` with kitty protocol), each press accumulates lines into the selection instead of replacing it. `o` in extend mode flips anchor/head (Vim visual `o`). `mil`/`mal` text objects still available for inner/around line. `x` was freed up from its earlier extend-mode role when the toggle was moved to `e`. |
| Delete char at cursor | **`d` (no separate binding needed)** | In select-then-act with always-inclusive 1-char selections, `d` (delete selection) on a fresh cursor deletes the char under it — identical to Vim's `x`. No dedicated binding required. `delete_char_forward` in `src/edit.rs` is for insert mode (the Delete key), not normal mode. |
| Terminal I/O | **crossterm** | Cross-platform terminal I/O. Handles raw mode, key events, escape sequences. |
| Rendering | **ratatui as diffing engine** | Use ratatui's `Buffer`/`Terminal` for cell-level rendering and double-buffer diffing. No widgets. Immediate mode thinking with retained-mode optimization. |
| Terminal protocol | **Prefer kitty keyboard, fall back** | Detect kitty keyboard protocol support at startup via a direct TTY probe (`src/os/`): sends `\x1B[?u` + `\x1B[>q` (XTVERSION) + `\x1B[c` (DA1 sentinel) and reads raw bytes from `/dev/tty` (Unix) or Win32 console API (Windows); detects kitty via flags response or XTVERSION name match (fallback for older WezTerm). Use it when available for unambiguous key encoding, modifier reporting, and key release events. Fall back to legacy encoding otherwise (like Helix does). |
| Documentation | **Markdown + auto-generated command reference** | Hand-written Markdown guides for concepts. Command reference auto-generated from Rust doc comments. In-editor `:help` renders Markdown in a read-only buffer. |
| Theming | **Hierarchical scopes** (Helix-compatible) | Dot-notation scopes (`keyword.function`, `ui.cursor`) with automatic fallback. Follow Helix scope convention. Read Helix TOML themes natively; Steel themes as primary format. Discoverability via `:theme-debug` and token inspection. |
| Package manager | **Built-in, declarative, replaceable** | Config declares plugins (`username/repo`). `:plugin-sync` reconciles disk to config (install/update/remove). Git-based, no registry, no auto dependency resolution. Built in Steel, replaceable by users. |
| Indent queries | **Helix format** (`indent.scm`) | Reuse Helix's existing per-language indent queries directly. No drawbacks identified; avoids reinventing a query format and gives us a large library of languages for free. |
| Unicode handling | **Grapheme clusters from day one** | All motions and selections operate on grapheme clusters via `unicode-segmentation`, not bytes or chars. Handles emoji, combining characters, CJK wide chars correctly. Avoids painful retrofitting. |
| Symbol rename | **LSP-first, tree-sitter fallback** | Use `textDocument/rename` when an LSP server is active — it is scope-aware and works across files. Fall back automatically to a tree-sitter local rename (using `locals.scm` scope queries) when no LSP is available — file-local only, but still scope-correct within the file. Same keybinding in both cases; the degraded fallback is transparent to the user. |
| Keymap defaults vs config | **Hardcoded defaults, config overrides** | Default keybinds live in Rust as the source of truth. User config (Steel) provides overrides only — not a full copy of all bindings. The editor always works with zero config: missing config file → silent, use defaults. Unparseable config → start with defaults, show warning in status line. Invalid individual entries → skip and warn, load the rest. New commands get their default keybind automatically on upgrade without requiring users to update their config. Principle: the editor must always be usable; config is an additive layer, never a requirement. |
| Register linewise flag | **Heuristic: detect at paste time** | Whether yanked content is linewise (whole lines) vs charwise is determined at paste time: if every value in the register ends with `\n`, treat as linewise. No explicit flag stored. Promote to an explicit flag later if the heuristic proves insufficient. |
| Paste on selection (`p`/`P`) | **Replace-and-swap, no separate `R` binding** | `p`/`P` on a cursor (1-char selection) inserts normally. `p`/`P` on a multi-char selection replaces it with the register contents, and the displaced text is returned to the caller to write back to the register (swap). This solves the yank-then-delete-then-paste clobber problem without Vim's `"0` yank register or a dedicated `R` keybind. The distinction is `sel.is_cursor()` — intentional selections trigger replace; fresh cursors trigger insert. |
| Register `'c'` (system clipboard) | **Deferred to M3 (editor layer)** | The `'c'` register requires OS clipboard integration (e.g. `arboard` crate) and belongs in the editor layer, not the core. The `CLIPBOARD_REGISTER` constant reserves the name; actual clipboard read/write will be wired in M3. For now it behaves like any named register. |
| Read-only registers (`.`, `%`, `#`) | **Deferred, editor-layer concern** | Last inserted text (`.`), current filename (`%`), and alternate filename (`#`) require editor-level state (mode tracking, open file list). Not implementable in the core layer. Deferred entirely until M3. |
| Register naming scheme | **Mnemonic letters, `0`–`9` named storage** | HUME uses mnemonic single-char names rather than Vim/Helix convention (`"`, `+`, `_`). 10 named registers (`0`–`9`) cover all real workflows, freeing letters for intuitive special names: `c` = clipboard, `b` = black hole, `s` = search, `q` = default macro. The default register (receives all yanks/deletes implicitly) is an internal sentinel (`'"'`) users never type. Named registers also store macros (Vim model); `0`–`9` hold text or keystrokes, last write wins. |
| Macro model | **Register-based, Vim-style, with `q`/`Q` UX** | Macros are stored in registers (Vim model), not in a single global slot (Helix model). Register `q` is the default macro register. `qq` starts/stops recording into `q`; `Q` replays from `q`. `q3` records into register `3`; `Q3` replays from `3`. Allows multiple saved macros without the full `a`–`z` Vim namespace. Implemented in M3 (editor layer). |
| Register picker UI | **Deferred to M3 (editor layer)** | When the user presses the register prefix, show a popup listing all registers with descriptions and current contents (like Helix). Makes register names discoverable without memorisation. The naming scheme is learnable but the picker removes the need to memorise it upfront. |
| Dot-repeat scope | **Action only, no preceding selection** (Helix/Kakoune model) | `.` replays the editing command + insert keystrokes, but NOT the motion/selection that preceded it. On a collapsed cursor after `wc`+"foo"+Esc, `.` deletes the single cursor char and inserts "foo" (not a full word). The user re-selects before pressing `.` (e.g. `w.w.w.`). This matches the select-then-act philosophy: selections and actions are independent steps. Vim's `cw`-style atomic operator+motion recording would require an operator-pending mode, which contradicts the model. Evaluated and rejected: making `.` a no-op on collapsed cursors would break legitimate `d..` (delete successive chars). |
| Word motions (`w`/`b`/`W`/`B`) | **Select whole word (not Helix extend model)** | `w` jumps to the NEXT word and selects it entirely (anchor = first char, head = last char). `b` jumps to the PREVIOUS word and selects it entirely. This keeps the select-then-act model clean: every `w`/`b` press gives a fresh, cleanly-bounded word selection. `e`/`E` are removed as redundant (since `w` already selects to the word end). `w` and `b` cross line boundaries (newlines). `w` on the last word in the buffer and `b` on the first word are no-ops (stay). This diverges from Helix, where `w` extends from the current position to the start of the next word. |

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
| **iTerm2** | macOS | yes | yes (3.5+) | yes | Most popular macOS terminal |
| **Windows Terminal** | Windows | yes | partial | yes | Default Windows terminal |
| **Terminal.app** | macOS | yes (Ventura+) | no | no | Apple's built-in — limited |
| **GNOME Terminal** | Linux | yes | no | yes | Common Linux default |
| **Konsole** | Linux | yes | no | yes | KDE default |
| **tmux** | cross-platform | yes (with config) | **no** | yes (3.3+) | Multiplexer — sits between emulator and app; does not pass through kitty protocol |

## Open Questions

| Question | Context |
|----------|---------|
| Multiline quote text objects | Quote text objects (`i"`, `i'`, `` i` ``) are line-bounded because the parity scan gives wrong results when earlier lines contain unmatched quotes. Brackets don't have this problem (asymmetric delimiters allow depth tracking). Tree-sitter can resolve the ambiguity — use syntax-aware matching when a grammar is loaded, fall back to line-bounded parity otherwise. |
