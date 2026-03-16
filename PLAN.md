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
| Testing | `cargo test` + crates | Built-in unit/integration/doc tests. Add `pretty_assertions`, `proptest`, `insta` as needed. |

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

## Testing Strategy

Every editing command, text object, and selection operation must be tested. Approach by layer:

- **Core editing tests (M1)**: Helix-style state triples — `(initial_state, operations, expected_state)` with a compact DSL using markers for cursor and selection (e.g. `#[hello|]# world`). Fast, focused, self-documenting. No UI dependency.
- **Renderer tests (M2+)**: `insta` inline snapshots — implement `render_to_string()` producing ASCII with cursor/selection markers. Expected output embedded directly in test source (`@"..."`). Auto-updateable via `cargo insta review`.
- **Property-based tests** (`proptest`): Buffer integrity invariants — random sequences of insert/delete/undo/redo must never corrupt the rope or desync selections.
- **Integration tests** (`tests/` directory): End-to-end editing sequences — open file, perform edits, verify final state.
- **`pretty_assertions`**: Better diff output for string/buffer comparisons in all test types.

## Milestones

### M0 — Bootstrapping (complete)
- [x] Project vision and README
- [x] Language decision: Rust
- [x] Decide on core libraries: crossterm + ratatui (diffing engine only)
- [x] Decide on text storage data structure: rope via `ropey`
- [x] Decide on editing model: Helix-style select-then-act
- [x] Initialize Rust project with Cargo

### M1 — Core engine
Build the core with no UI dependency. Drive entirely from tests.
- [x] Buffer type: wrap `ropey::Rope` with HUME's buffer API
- [x] Selection type: `Vec<Selection>` with anchor + head (always inclusive — `anchor == head` is a 1-char selection, never a zero-width point). Single cursor is a vec of length 1
- [x] Unicode/grapheme cluster handling: all motions and selections operate on grapheme clusters (`unicode-segmentation` crate), not bytes or chars
- [x] Basic edit operations: insert, delete, backspace — operating over all selections
- [x] ChangeSet: OT-style edit descriptions (Retain/Delete/Insert) with apply, map_pos, invert, compose. Builder pattern for constructing changesets. Edit operations refactored to build changesets.
- [x] Transaction: thin wrapper pairing ChangeSet with SelectionSet — the unit of editing for future undo/redo
- [x] Motions: character, word, line, paragraph movement — implemented as named commands (`src/motion.rs`); key bindings are wired in M3. Extend variants exist as named commands (e.g. `cmd_extend_next_word_start`). Key-to-command mapping is an M3/keybinding concern.
- [x] Text objects: inside/around word, quotes, brackets, line
- [ ] Selection manipulation: extend, shrink, split, merge overlapping selections
- [ ] Registers: named yank/paste buffers (`"a`–`"z`) including system clipboard register (`"+`)
- [ ] Count prefix: numeric prefix to repeat motions/actions (`3w`, `5x`)
- [ ] Repeat last command (`.` equivalent)
- [ ] Undo/redo: tree-structured undo with changesets
- [ ] Property-based tests (`proptest`): random edit sequences never corrupt buffer or desync selections
- [ ] Thorough unit tests for every operation and edge case

### M2 — First render
- [ ] Display-line abstraction (buffer line or virtual line)
- [ ] Open and display a file with scrolling
- [ ] Line numbers
- [ ] Status bar with filename and position
- [ ] Quit command

### M3 — Modal editing
- [ ] Normal mode with cursor movement (drives core motions)
- [ ] Insert mode with text input
- [ ] Command mode (`:` commands)
- [ ] Keymap: command-based dispatch from Steel config
- [ ] Extend mode: `x` toggles extend mode (all terminals); `Ctrl+motion` as extend shortcut when kitty keyboard protocol is active. In extend mode all motions extend the current selection instead of replacing it. Ctrl rejected as universal modifier due to fatal legacy-terminal collisions.
- [ ] Line selection: needs a key binding (not `x` — taken by extend mode; not yet decided)
- [ ] Paragraph navigation: bind `{` / `}` (prev/next paragraph). Free in both Helix and Kakoune. Preferred over Helix's `[p` / `]p`.
- [ ] Auto-pairs: auto-close brackets, quotes (configurable)
- [ ] Matching bracket highlight
- [ ] Cursor line highlight

### Future milestones
- **Motion selection model**: decide between Helix (jump/collapse) and Kakoune (select-through) for paragraph and goto-line motions. Currently Helix. One-word change per command (`MotionMode::Move` → `Select`). Decide after trying both in the running UI.
- Tree-structured undo/redo (vim-style undo tree)
- Search and replace with incremental search and live match highlighting
- File picker / fuzzy finder (Helix-style picker with fuzzy matching)
- Jump list (navigate back/forward through cursor position history)
- Surround operations (add/change/delete surrounding characters)
- Multiple buffers / splits
- Syntax highlighting via tree-sitter
- Tree-sitter structural features: text objects (`locals.scm`, `textobjects.scm`), scope-aware local rename (fallback when LSP unavailable)
- Soft wrap (option to wrap long lines vs horizontal scroll)
- Code folding (tree-sitter powered collapse/expand)
- Mouse support (click to position cursor, scroll, basic selection)
- Git gutter signs (added/modified/deleted line indicators)
- Whitespace rendering (show tabs, trailing spaces, etc.)
- File watcher (detect external file changes, prompt to reload)
- Steel scripting engine + plugin API
- Configuration via Steel
- LSP support (Rust transport + Steel behavior layer): completions, diagnostics, hover, go-to-definition, `textDocument/rename` (falls back to tree-sitter local rename when LSP unavailable)
- Virtual lines / decoration layer (inline diagnostics, ghost text, code lenses, inlay hints)
- Documentation: Markdown guides, auto-generated command reference, in-editor `:help`
- Theming: Hierarchical scopes (Helix-compatible), Steel + Helix TOML theme formats, `:theme-debug`
- Package manager: Declarative sync model, git-based, `:plugin-sync` / `:plugin-update` / `:plugin-status`
