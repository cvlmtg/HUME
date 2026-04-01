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
- **UI**: Tab bar, status line, command line, split panes. The status line follows an **element model** (`StatusElement` enum in `src/ui/statusline.rs`): named elements (`Mode`, `Position`, `FileName`, `Separator`, `DirtyIndicator`, `SearchMatches`, `KittyProtocol`, `MiniBuf`, …) arranged into left / center / right sections via `StatusLineConfig`. The renderer adds edge padding (1-space margin) and boundary-aware spacing between elements. The Steel config layer will expose this to scripting.
- **Decorations**: Annotation layer for virtual lines/text (diagnostics, ghost text, code lenses, inlay hints, git blame). Buffer-position-anchored, auto-updated on edits, queryable by line. Multiple sources (LSP, plugins, git).
- **Scripting**: Steel (Scheme) engine for plugins and configuration
- **LSP**: Rust transport/parsing layer + Steel scripts for behavior and customization

## Testing Strategy

Every editing command, text object, and selection operation must be tested. Approach by layer:

- **Core editing tests (M1)**: Helix-style state triples — `(initial_state, operations, expected_state)` with a compact DSL using markers for cursor and selection (e.g. `-[hello]> world` for forward, `<[hello]- world` for backward). Fast, focused, self-documenting. No UI dependency.
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
- [x] Transaction: thin wrapper pairing ChangeSet with SelectionSet — the unit of undo/redo
- [x] Motions: character, word, line, paragraph movement — implemented as named commands (`src/motion.rs`); key bindings are wired in M3. Extend variants exist as named commands (e.g. `cmd_extend_next_word_start`). Key-to-command mapping is an M3/keybinding concern.
- [x] Text objects: inside/around word, quotes, brackets, line
- [x] Selection manipulation: collapse, flip, keep/remove/cycle primary, split on newlines, copy to adjacent line, trim whitespace (`selection_cmd.rs`)
- [x] Registers: named yank/paste buffers (`'0'`–`'9'`) + default (`'"'`) + black hole (`'b'`) + reserved slots for clipboard (`'c'`), search (`'s'`), macro (`'q'`); `yank_selections` (`src/register.rs`), `paste_after`/`paste_before` (`src/edit.rs`). System clipboard deferred to M4 (editor layer).
- [x] Count prefix: numeric prefix to repeat motions/actions (`3w`, `5x`)
- [x] Undo/redo: tree-structured undo with changesets (`History` arena in `src/history.rs`, `Document` orchestrator in `src/document.rs`)
- [x] `goto_revision`: jump to any node in the undo tree directly (`History::goto_revision` + `Document::goto_revision`); uses LCA-based path-finding, applies inverse/forward transactions sequentially
- [x] Property-based tests (`proptest`): random edit sequences never corrupt buffer or desync selections
- [x] Thorough unit tests for every operation and edge case

### M2 — First render ✓
- [x] Display-line abstraction (buffer line or virtual line)
- [x] Open and display a file with scrolling
- [x] Line numbers (absolute / relative / hybrid)
- [x] Status bar with filename and position
- [x] Quit command

### M3 — Modal editing ✓
- [x] Normal mode with cursor movement: `h/l/j/k`, arrows, `w/b/W/B` (select whole word), `Home/End/0/$`, `^` (first non-blank), `{`/`}` (paragraph), `PageUp/PageDown`, `;` (collapse), `,` (keep primary), `(`/`)` (cycle primary), `C` (copy to next line), `d` (delete + yank), `c` (change + yank), `u/U/Ctrl+r` (undo/redo), `i/a` (enter Insert), `q/Ctrl+c` (quit)
- [x] Yank/paste: `y` (yank), `p` (paste after), `P` (paste before); `d`/`c` yank before deleting; paste on non-cursor selection swaps displaced text back into default register
- [x] Text objects: `mi`/`ma` + object char — word (`w`/`W`), brackets (`(`/`[`/`{`/`<`), quotes (`"`/`'`/`` ` ``); unrecognized char falls through to normal dispatch
- [x] Insert mode with text input: `Esc` to return to Normal; character input, `Enter`, `Backspace`, `Delete`; arrow keys and `Home/End` for navigation
- [x] Extend mode: `e` toggles sticky extend mode (`EXT` status bar label); all motions and text objects extend/union the selection instead of replacing it. Word motions use union semantics (selection grows to encompass the next/prev word). `Ctrl+motion` one-shot extend shortcuts deferred to M4 (requires kitty keyboard protocol).
- [x] Cursor line highlight: subtle background on the cursor row in `render_content`; `cursor_line` already computed in `render()`.
- [x] Line selection: `x` selects the full current line (including `\n`); repeated `x` walks to the next line. `X` selects the current line backward; repeated `X` walks upward. `Ctrl+x` / `Ctrl+X` (kitty-only) accumulate lines downward/upward without replacing the selection. Extend mode (`e`) activates the same accumulation semantics. `mil`/`mal` text objects still available for inner/around line via `dispatch_text_object`.
- [x] Command mode (`:` commands): `Mode::Command`, mini-buffer input, command-line row in renderer, parser for `:q`/`:w`/`:wq`, file write. Replaces temporary `q`-to-quit.
- [x] Matching bracket highlight: `find_bracket_pair` in `text_object.rs`; `HighlightSet` in `src/highlight.rs` (sorted vec + binary search, `&'static EMPTY` for zero-allocation Insert mode path); bracket pair computed each frame in `editor/mod.rs`, passed into `RenderCtx`.
- [x] Auto-pairs: auto-close brackets/quotes on insert; self-contained, no ordering pressure.
- [x] f/t/F/T character find motions: `f`/`F` (inclusive), `t`/`T` (exclusive); `=`/`-` repeat with absolute direction (always forward/backward, regardless of original f/F/t/T).

### M4 — Command architecture + search

Theme: replace hardcoded key dispatch with a proper command registry and keymap layer, then add the highest-value editing features that depend on it.

- [x] **Kitty keyboard protocol** (`src/terminal.rs`, `src/os/{unix,windows}.rs`, `Editor::kitty_enabled`): probe at startup with `crate::os::probe_kitty_support()`: sends `\x1B[?u` (kitty flags query), `\x1B[>q` (XTVERSION), and `\x1B[c` (DA1 sentinel) to the terminal, then reads raw response bytes directly from `/dev/tty` (Unix) or the Win32 console API (Windows) — bypasses crossterm's event system entirely to avoid startup timeouts; detects kitty via `ESC [ ? <digits> u` response (flags query) or XTVERSION name matching as a fallback for terminals that support push but not the query (e.g. older WezTerm); push `DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES` flags when supported; pop unconditionally in `restore()` (harmless no-op on legacy terminals); filter `KeyEventKind::Release` in the event loop; store `kitty_enabled: bool` on `Editor`; add Ctrl+h/l/j/k/w/b one-shot extend bindings gated on `kitty_enabled`; status bar shows 🐱 when kitty protocol is active. Graceful fallback to legacy encoding when the terminal doesn't support the protocol.
- [x] **Command registry** (`src/editor/registry.rs`): typed command descriptors behind string names — `Motion`, `Selection`, and `Edit` variants; `register_defaults()` registers every `cmd_*` function. The `cmd_*` signatures are already the right shape.
- [x] **Keymap layer** (`src/editor/keymap.rs`): trie-based `KeyEvent` sequence → command name mapping; per-mode keymaps (Normal, Insert); replaces the `PendingKey` enum and all `handle_normal` match arms. Default keymap includes `Ctrl+motion` one-shot extend variants enabled by kitty protocol. Default keymap in Rust (Steel config is M5).
- [x] **Goto commands** (`g` prefix): `gg` (first line), `ge` (last line), `gh` (line start), `gl` (line end), `gs` (first non-blank) — validates the multi-key trie.
- [x] **Repeat last command (`.`)**: `repeatable: bool` flag on each `MappableCommand` variant; `RepeatableAction` stores command name + count + char arg + insert keystrokes; `insert_recording` buffer captures text-input keys during insert sessions; `cmd_repeat` re-executes through the normal dispatch path then replays insert keys.
- [x] **File save robustness**: `:w <path>` save-as, "no file name" error, dirty-buffer tracking in status bar (builds on M3 command mode). `:q` guard warns on unsaved changes; `:q!` force-quits. Dirty state is revision-based (`saved_revision` in `Document` vs `History::current_id()`) so undo back to save point correctly reports clean. `DirtyIndicator` segment in status bar shows `[+]`. Command parsing split into name + arg + force flag.
- [x] **Incremental search** (`/` and `?`): `Mode::Search { direction }`, reuses command-mode mini-buffer; live match highlighting via `HighlightSet` (`src/highlight.rs` — already exists, push match ranges each frame); `n`/`N` repeat; `Esc` restores position; pattern stored in `'s'` register.
- [x] **Search-based selection** (`*` and `s`): `*` uses current selection as search pattern (expands to word under cursor for single-char selections); `s` enters Select mode — prompts for a regex, all matches within current selections become new selections (live preview). Combined with `c`, this gives search-and-replace via multi-cursor.
- [x] **Jump list**: Vec+cursor of cursor positions; record on search jumps, goto, and motions > 5 lines; `Ctrl-o` / `Ctrl-i` (`Tab`) navigate.
- [x] **Surround operations** (`ms` + smart `r`): `ms` + char selects the surrounding delimiters as two cursor selections (e.g. `ms(` places cursors on `(` and `)`), enabling select-then-act composition: `ms(` → `d` deletes parens, `ms(` → `r[` replaces `()` with `[]`, `ms(` → `c` enters insert with two cursors. No separate `md`/`mr` — standard commands compose naturally. Smart `r`: when replacing cursor selections with a pair character, resolves open/close based on the char being replaced (opening→opening, closing→closing; symmetric delimiters use selection index as tiebreaker). "Add surround" is already covered by auto-pairs wrapping in insert mode.

### M5 — Scripting foundation + polish

Theme: embed the Steel scripting engine and land the most impactful editing polish features. Tree-sitter is deferred to M6.

- [ ] **Whitespace rendering**: configurable visual indicators for spaces, tabs, trailing whitespace. `WhitespaceShow` enum (`None`/`Trailing`/`All`), custom indicator characters, dimmed style. Pure renderer change.
- [ ] **Keyboard macros**: register-based recording and replay. `qq` starts/stops recording into register `q`; `Q` replays from `q`. `q3` records into register `3`, `Q3` replays from `3`. `RegisterContent` enum (`Text`/`Keys`) — registers hold text or keystrokes, last write wins. Count prefix on replay (`3Qq`). Status bar shows `[recording @q]`. `replaying_macro` flag suppresses nested recording.
- [ ] **Soft wrap**: long lines wrap to the next display row. `DisplayLine` gains `is_continuation: bool`. `display_lines()` splits lines exceeding `content_width` into multiple display rows. `scroll_offset` stays buffer-line based. `col_offset` forced to 0 when wrapping. `j/k` remain buffer-line motions (display-line motions deferred). Continuation rows show wrap indicator in gutter.
- [ ] **Mouse support**: click to position cursor, drag to select, scroll wheel. `EnableMouseCapture` via crossterm. Core challenge: `screen_to_buffer_pos` mapping (screen column → grapheme walk → buffer char offset). Handles CJK double-width and soft-wrapped continuation lines. `mouse_anchor` on `Editor` for drag selection.
- [ ] **Steel scripting engine + plugin API**: embed `steel-core`. `ScriptEngine` wrapper with `Option` extraction pattern (Helix-style ownership). API: `keymap-bind!`/`keymap-unbind!` for keymap overrides, `define-command!` for Steel-implemented commands, `set-option!` for configuration. Config loaded from `$XDG_CONFIG_HOME/hume/init.scm` at startup. Missing config → silent. Parse error → defaults + warning. Invalid entries → skip + warn. `CommandRegistry` gains `steel_commands` map and mutable registration API. `KeymapCommand.name` changes to `Cow<'static, str>` for runtime names. `jump: bool` field on `MappableCommand` replaces `JUMP_COMMANDS` const.
- [ ] **Configuration via Steel**: keymap overrides, whitespace/soft-wrap/line-number options, theme color overrides. Additive layer — user config specifies only overrides, defaults always work.
- [ ] **Configurable status line**: expose `StatusLineConfig` to Steel — left/center/right section lists accept element names (keywords) or arbitrary Steel functions `(fn [ctx] string)`. Built-in elements already implemented: `Mode`, `FileName`, `Position`, `Separator`, `DirtyIndicator`, `SearchMatches`, `KittyProtocol`, `Selections`, `MiniBuf`, `MacroRecording`. Future elements: `file-path`, `position-percentage`, `file-encoding`, `file-line-ending`, `file-type`, `diagnostics`, `version-control`, `register`, `spacer`.
- [ ] **Helix-compat surround plugin** (Steel): `md` + char deletes surround, `mr` + old + new replaces surround. Implemented as a bundled Steel plugin that composes existing commands (`select-surround` → `delete` / `replace`). Serves as the first real proof-of-concept for the scripting engine and validates that the plugin API is expressive enough for command composition.

### M6 — Syntax awareness (planned)

- **Syntax highlighting via tree-sitter**: grammar loading, parse-on-edit pipeline, highlight spans in renderer.
- **Tree-sitter structural features**: text objects (`locals.scm`, `textobjects.scm`), scope-aware local rename (fallback when LSP unavailable).

### Future milestones
- **Register paste count mismatch**: when yank uses N cursors but paste uses M≠N, Helix falls back to pasting the full register at every cursor. Explore alternatives with real usage data (e.g. cycling slots, clamping to last slot, user-facing warning). Decide after more real usage.
- **Multiple buffers / splits**: large layout/architecture work; single-document model is fine until then.
- **File picker / fuzzy finder** (Helix-style picker): depends on multiple buffers.
- **LSP support** (Rust transport + Steel behavior layer): completions, diagnostics, hover, go-to-definition, `textDocument/rename`. Depends on Steel, tree-sitter, multiple buffers.
- **Virtual lines / decoration layer** (inline diagnostics, ghost text, code lenses, inlay hints): depends on LSP.
- Code folding (tree-sitter powered collapse/expand)
- Git gutter signs (plugin candidate — keep out of core, implement via Steel once scripting lands)
- File watcher (detect external file changes, prompt to reload)
- Documentation: Markdown guides, auto-generated command reference, in-editor `:help`
- Theming: Hierarchical scopes (Helix-compatible), Steel + Helix TOML theme formats, `:theme-debug`
- Package manager: Declarative sync model, git-based, `:plugin-sync` / `:plugin-update` / `:plugin-status`
