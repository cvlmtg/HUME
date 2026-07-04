# HUME - Roadmap

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | **Rust** | Memory-safe, zero-cost abstractions, best TUI ecosystem (crossterm + ratatui). Expressive type system with pattern matching, algebraic types, and macros. Ideal learning project. |
| Text storage | **Rope** (via `ropey`) | Efficient for large files, O(log n) edits anywhere, built-in line indexing. Used by Helix. |
| Scripting / Config | **Steel** (Rust-native Scheme) | Lisp syntax, designed for embedding in Rust. Helix is adopting it. Used for both plugins and configuration. |
| Scripting crate boundary | **`EditorHost` trait inversion + `hume-scripting/` crate** | Severs editor-domain coupling from the Steel host via trait inversion. `hume-platform/` hosts terminal I/O and OS helpers. |
| Editing model crate | **`hume-editing/` crate** | Pure bottom layer (no engine dependency) for the foundational text data model. Named `hume-editing` to avoid shadowing Rust's built-in `core`. |
| LSP architecture | **Hybrid** | Rust core handles transport and JSON-RPC parsing. Steel scripts handle behavior (diagnostics display, completion UX, keybindings). |
| Syntax highlighting | **Tree-sitter** | Incremental parsing, structural understanding. Enables text objects and structural navigation beyond just colors. Production-proven (Neovim, Helix, Zed, GitHub). |
| Key mapping | **Command-based** | Keys bind to named commands, not to other keys. No recursive/non-recursive distinction. Keymaps defined in Steel config. Supports nested keys for sequences/chords. |
| Editing model | **Select-then-act** | Motions create selections, actions operate on them. Multiple selections from day one (`Vec<Selection>`). Selections always inclusive — `anchor == head` is a 1-char selection, never a zero-width point. |
| Extend mode | **`e` toggle (primary) + Ctrl+motion (kitty bonus)** | Sticky mode; `e` mnemonic. Ctrl+motion rejected as universal modifier: fatal legacy-terminal collisions on 10 of 15 motion keys (`Ctrl+h`=Backspace, `Ctrl+[`=ESC, etc.). Alt rejected — types accented chars on macOS, physical layout issues on Windows. Kitty Ctrl+motion is a graceful bonus; explicit `Ctrl+letter` leaves use a `force_extend` flag instead. |
| Line selection | **`x`/`X` (walk down/up lines)** | `x` selects current line including `\n`; repeated `x` walks to next. `Ctrl+x`/`Ctrl+X` accumulate lines (force_extend, works in both kitty and legacy). `o` in extend mode flips anchor/head. Extend is bidirectional — see next row. |
| Extend direction (`w`/`b`, `x`/`X`) | **Bidirectional: anchor-unit for words, anchor-line span for lines** | Opposite-direction presses shrink the selection back instead of only ever growing (Helix/Kakoune's extend only grows). Words: the anchor's word is re-derived fresh from its current position on every press and always kept whole — crossing it flips the selection's direction rather than truncating the word. Lines: an unaligned selection is first aligned to full lines (direction fixed by which key was pressed), then each press moves the *head*'s line by ±1 relative to the anchor's line; clamps at the buffer's first/last line are head-relative, not selection-end-relative. |
| Delete char at cursor | **`d` (no separate binding)** | In select-then-act with always-inclusive 1-char selections, `d` on a fresh cursor deletes the char under it — identical to Vim's `x`. |
| Terminal I/O | **crossterm** | Cross-platform. Handles raw mode, key events, escape sequences. |
| Rendering | **ratatui as diffing engine** | Cell-level rendering via ratatui's double-buffer diffing. No widgets. Immediate-mode thinking with retained-mode optimization. |
| Terminal protocol | **Prefer kitty keyboard, fall back** | Probe at startup via direct TTY read. Use when available for unambiguous key encoding and modifier reporting. Fall back to legacy encoding otherwise. |
| Documentation | **Markdown + auto-generated command reference** | Hand-written Markdown guides for concepts. Command reference auto-generated from Rust doc comments. In-editor `:help` renders Markdown in a read-only buffer. |
| Theming | **Hierarchical scopes** | Dot-notation scopes (`keyword.function`, `ui.cursor`) with automatic fallback. Helix scope names for theme compatibility. Helix TOML theme format including `inherits` chains and `palette`. |
| Package manager | **PLUM (PLUgin Manager) — Steel script, swappable** | Core plugin (`core:plum`). Two namespaces: `core:name` (bundled) and `username/repo` (Git-based, no registry). Every plugin must appear in `init.scm` via `(load-plugin …)` or `(declare-plugin …)`. Disabling PLUM just removes management commands. |
| Lazy plugin loading | **Plugin manifests; load-once (no unload)** | `(load-plugin name)` = eager; `(declare-plugin name …)` = lazy manifest declaring `#:commands` / `#:events` / `#:languages` activation entries. Fail-fast on first error; no retry until `:reload-config`. No unload. |
| Plugin namespace isolation | **One Steel module per plugin (`require`)** | Each plugin loads via `(require "<abs-path>")` giving its own module namespace. Cross-plugin calls go through `call!` only. |
| Plugin attribution / lifecycle | **Keep attribution, drop ledger; fail-fast** | `PluginId` / `Owner` / `PluginStack` / `cmd_owners` track who registered what. Mutation ledger and per-plugin rollback removed — Neovim/Helix ship no clean per-plugin unload. `:reload-config` rebuilds from scratch. |
| Command dispatch args | **Variadic `call!`; no side channel** | `(call! name args…)` dispatches to Steel or native commands. Arity-based injection: 0 params → nothing, 1 → count, 2/variadic → count+extend. |
| Indent queries | **Helix format (`indent.scm`)** | Reuse Helix per-language indent queries directly. Avoids reinventing a query format; gives a large library of languages for free. |
| Unicode handling | **Grapheme clusters from day one** | All motions and selections use `unicode-segmentation`, not bytes or chars. Retrofitting is expensive. |
| Symbol rename | **LSP-first, tree-sitter fallback** | `textDocument/rename` when LSP active; falls back to tree-sitter local rename via `locals.scm` — file-local only but scope-correct. Same keybinding in both cases. |
| Keymap defaults vs config | **Hardcoded defaults, config overrides** | Defaults in Rust as SSOT. User config (Steel) provides overrides only. Editor always works with zero config; missing or unparseable config silently uses defaults. |
| Jump-list eligibility | **Flag on `MappableCommand`** | `jump: bool` field (like `repeatable`) lets every command — Rust or Steel — declare jump intent at registration. A `const` list can't capture plugin commands that move the cursor directly. |
| Register linewise flag | **Heuristic: detect at paste time** | Trailing `\n` signals linewise content. No explicit flag stored; natural upgrade path if heuristic proves insufficient. |
| Paste on selection (`p`/`P`) | **Replace selection, no swap** | `p`/`P` on cursor inserts normally; on a multi-char selection, replaces it. Displaced text not written back — kill ring gives access to previous content. |
| Register `'c'` (clipboard) | **Editor layer** | Requires OS clipboard integration; belongs in editor layer, not core. |
| Read-only registers (`.`, `%`, `#`) | **Editor layer** | Require editor-level state (mode tracking, open file list); not implementable in core. |
| Register naming | **Mnemonic letters, `0`–`9` named storage** | `k` = kill-ring head, `c` = clipboard, `b` = black hole, `s` = search, `q` = default macro. 10 named registers (`0`–`9`) for symmetric in-memory storage. Default register (`"`) is internal. Kill ring reached via `"k` or `[`/`]` cycling. |
| Macro model | **Register-based, Vim-style, with `Q`/`q` UX** | Macros stored in registers. `q` = default. `QQ` records into `q`; `qq` replays. `Q` for record (deliberate setup); `q` for replay (hot path). Multiple saved macros without the full `a`–`z` Vim namespace. |
| Register picker UI | **Editor layer** | Popup listing registers with descriptions and contents when register prefix pressed. Makes register names discoverable without memorisation. |
| Dot-repeat scope | **Selection recipe + action** | Replays: last selection-establishing command + extend steps + editing command + insert keystrokes. Plain navigation not recorded. Recipe discriminated by `Selection::is_collapsed()` after each Motion/Selection command. Steel commands opt in via `#:repeatable #t`. Mutually exclusive with `#:inline-output`. |
| Surround operations | **`ms` + smart `r` (select-then-act, no `md`/`mr`)** | `ms` + char selects surrounding delimiters as two cursor selections. Delete = `ms(` → `d`. Replace = `ms(` → `r[`. Smart `r` maps open→open, close→close; symmetric delimiters use selection index as tiebreaker. Rejected: Helix `md`/`mr` which bake selection+action together, violating select-then-act. |
| Word motions (`w`/`b`/`W`/`B`) | **Select whole word** | `w` selects the next word entirely; `b` selects the previous word entirely. `e`/`E` removed as redundant. Diverges from Helix where `w` extends to start of next word. |
| Multi-buffer scope (M7) | **List + switch only; no splits or tab UI** | Multiple open buffers, `:e`/`:bnext`/`:bprev`/`:bd`. Splits and tabline deferred — require a layout engine. |
| Disjoint-borrow refactor | **Logic in free functions over sub-struct borrows, not `&mut self` methods on outer `Editor`** | `Editor` is a thin wrapper over three sibling sub-structs — `scripting`, `state` (`EditorState`, all command-mutable data), and `view` (`EngineView`). The split exists so command/editing logic can take the narrowest borrow (`&mut EditorState`, `&EngineView`, or individual fields) as a free function, letting Rust prove borrows disjoint instead of forcing workaround clones. A `&mut self` method on the *outer* `Editor` borrows all three at once, blocking that — such methods are reserved for lifecycle entry points, cross-cutting orchestration, and thin delegators. Enforcement is structural: the borrow checker rejects cross-sub-struct access from a sub-borrow, so a new whole-`Editor` borrow appears as an explicit clone or a new outer `&mut self` method in review — no separate lint. |
| Kitty push order on init | **Push after entering alt screen** | Some terminals maintain a per-screen keyboard stack. Pushing before `EnterAlternateScreen` lands flags on the primary screen's stack, which key reads don't consult. |
| SHIFT normalization | **Strip bare SHIFT for any Char** | Terminals that don't honour `REPORT_ALTERNATE_KEYS` send shifted punctuation as `Char + SHIFT`, missing trie bindings. Strip SHIFT for any `Char` when SHIFT is the only modifier (`Ctrl+Shift` keeps CONTROL; `Shift+Tab` = `BackTab` is not a `Char`). |
| Kill-ring whitespace collapse | **Overwrite pure-whitespace head in place** | On push, if the ring head is pure-whitespace, the new entry overwrites it instead of taking a fresh slot. Keeps swap-junk from filling the ring; the whitespace stays retrievable until the next push. |
| Tab handling | **One knob (`tab-style`), reuse `tab-width`** | Tab key inserts `\t` (hard) or spaces to next tab stop (soft). `tab-width` reused for both rendering and Tab-key spacing. Vim's four-knob `shiftwidth`/`softtabstop` model rejected as too complex. Enter copies leading whitespace verbatim. |
| Diff module | **histogram + Myers fallback, `similar` hidden** | `diff_lines`: histogram with wall-clock deadline, falls back to Myers on timeout. `diff_words`: Myers over UAX #29 word-boundary tokens with deadline. `similar` is a hidden impl detail — public types owned by `hume-editing` so consumers never depend on the backend. |
| `:split` / `:vsplit` direction mapping | **`:split` = top/bottom stack → engine `Direction::Vertical`; `:vsplit` = side-by-side → `Direction::Horizontal`** | Vim convention. Engine `Direction` partitions the named axis, so the enum name is the inverse of intuition — a named constructor/helper prevents bugs. |
| Split buffer | **Duplicate focused pane's buffer** | `:split`/`:vsplit` mirror the focused pane's buffer; optional `<path>` arg opens a different buffer. Matches Helix multi-pane contract. |
| Jump list on split | **Same-buffer split clones source; diverges thereafter** | A same-buffer `:split`/`:vsplit` clones the source pane's `JumpList` (entries + cursor) so the new pane can Ctrl+O back to pre-split positions. Different-buffer split starts empty. After the split each pane's jumps mutate independently. |
| Close-pane | **`Ctrl+p c` / pane-aware `:q`; no `:close`** | Prunes the focused leaf, collapses the parent `Split`, refocuses the sibling. `:q` closes the focused pane when others are open; only falls through to its existing buffer/quit logic on the sole pane. `Ctrl+p c` on the sole pane warns instead of quitting — `:q` alone owns quitting. |
| `wrap_mode` scope | **Moved onto `Pane`; `EditorSettings.wrap_mode` is init-default only** | Wrap is a view property, not a document preference. `BufferOverrides.wrap_mode` removed. Runtime per-pane override added via `:set pane wrap-mode=<value>` (2026-07-03) — `Pane` also carries `saved_wrap_mode` (always a wrapping variant) so `:wrap` toggle-on restores whatever mode the pane last wrapped with instead of hardcoding `indent`. Steel-side per-pane/per-buffer `set-option!` still deferred — only `Global` scope is scriptable. |
| `top_row_offset` vs virtual lines | **Counts wrap rows of `top_line` only; virtual rows never consume it** | `top_row_offset` is computed editor-side from `format::count_visual_rows`, which has no notion of virtual lines. `Before(top_line)` virtual rows above the scrolled-into window are dropped without decrementing the skip budget — consequence: a `Before` virtual block can't be scrolled partially off-screen, it disappears as a unit once the viewport scrolls into `top_line`. Fine-grained (virtual-aware) scroll math is future work. |
| Dynamic decoration content (engine) | **Per-frame text arena for cells; `Cow` for gutter; `Box<str>` for whitespace glyphs** | `CellContent::{Virtual,Indicator}` hold `{start: u32, len: u16}` ranges into `FormatScratch::virtual_texts` (cleared per buffer line/virtual row) instead of `&'static str`, so `Grapheme` stays `Copy` while decoration text (LSP hints, Steel icons) can be computed at runtime. `InlineInsert.text` and `VirtualLine::{text, segments}` are owned (`String`); the pipeline builds virtual-line graphemes itself from `text`+scoped byte-range `segments` rather than trusting provider-built `Grapheme`s. `GutterCellContent::Text(Cow<'static, str>)` replaces `Static`/`Number`. `WhitespaceConfig`'s three glyph fields are `Box<str>`. |
| Sign column tie-breaking (`SignColumn`) | **Higher `priority` wins; ties go to the later-registered source** | `SignColumn` merges `Sign`s from multiple `SignSource`s (diagnostics, git, breakpoints, bookmarks) into one narrow gutter column via `max_by_key(priority)`, which keeps the *last* max on ties — i.e. registration order breaks ties, not first-registered-wins. `GutterCell.scope` gained a `GutterScope::{Name(Scope), Id(ScopeId)}` split so `Sign` (and any future fast-path gutter provider) can carry an already-interned `ScopeId` like `HighlightSource`/`InlineInsert` do, instead of forcing every gutter column through the by-name resolve path. |

## Open Questions

| Question | Context |
|----------|---------|
| Multiline quote text objects | Quote text objects (`i"`, `i'`, `` i` ``) are line-bounded because the parity scan gives wrong results when earlier lines contain unmatched quotes. Brackets don't have this problem (asymmetric delimiters allow depth tracking). Tree-sitter can resolve the ambiguity — use syntax-aware matching when a grammar is loaded, fall back to line-bounded parity otherwise. |
| Register paste count mismatch | When yank uses N cursors but paste uses M≠N, Helix falls back to pasting the full register at every cursor. Explore alternatives with real usage data (e.g. cycling slots, clamping to last slot, user-facing warning). Decide after more real usage. |
| Common-prefix auto-extend on Tab | Readline/bash behaviour: on Tab with ≥2 candidates, first extend the input to their shared prefix before opening the popup. Current behaviour skips straight to cycling. Small UX polish; decide after more real usage whether the extra keystroke saved is worth the surprise of input mutating mid-type. |
| Plugin-defined languages × lazy loading | Language identity/detection is HUME-owned (`runtime/scheme/languages.scm`, eager). A plugin keying `#:languages` off an *existing* language works fine. But a plugin that defines its **own** language must register that identity **eagerly** — a lazy body cannot be the sole provider of its own activation language (chicken-and-egg). Deferred to a dedicated brainstorm. |
| `llvm-mir` / `llvm-mir-yaml` grammar mismatch | Inherited from Helix: `llvm-mir-yaml` defines the `.mir` extension but delegates to the `llvm-mir` grammar (no separate compiled grammar). `llvm-mir` itself has no extension binding. Track upstream; no HUME-specific fix needed until then. |
| Undercurl blocked on ratatui underline-shape support | `UnderlineStyle::{Wavy,Dotted,Dashed}` all collapse to plain `Modifier::UNDERLINED` in `From<ResolvedStyle> for ratatui::style::Style` (`types.rs`) — ratatui's `Modifier` bitflags have no underline-shape bits and its crossterm backend never emits `Undercurled`/`Underdotted`/`Underdashed`, though crossterm itself supports them. No clean injection point between the engine and the terminal today. Deferred: the engine model and theme loader are already correct, so this needs no engine change once ratatui adds underline-shape support — revisit then. |

## Milestones

### M0 — Bootstrapping (complete)
- [x] Project vision, language/library/data-structure/editing-model decisions, Rust project initialized

### M1 — Core engine (complete)
- [x] Buffer (rope wrapper), selections (`Vec<Selection>`, always-inclusive), grapheme cluster handling
- [x] Edit ops (insert/delete/backspace), ChangeSet (OT-style), Transaction (ChangeSet + SelectionSet)
- [x] Motions (char/word/line/paragraph), text objects (word/brackets/quotes/line), selection manipulation
- [x] Registers + kill ring, count prefix, undo/redo tree with `goto_revision`
- [x] Property-based and unit tests

### M2 — First render (complete)
- [x] Display-line abstraction, file open + scrolling, line numbers, status bar, quit

### M3 — Modal editing (complete)
- [x] Normal mode motions, yank/paste, text objects, insert mode, extend mode, cursor-line highlight
- [x] Line selection (`x`/`X`/`Ctrl+x`/`Ctrl+X`), command mode (`:q`/`:w`/`:wq`), bracket highlight, auto-pairs
- [x] `f`/`t`/`F`/`T` character find, `=`/`-` repeat

### M4 — Command architecture + search (complete)
- [x] Kitty keyboard protocol with legacy fallback
- [x] Command registry + trie keymap layer (Normal/Insert/Extend)
- [x] Goto commands (`g` prefix), dot-repeat (`.`), file-save robustness (`:w <path>`, dirty tracking, `:q!`)
- [x] Incremental search (`/`/`?`), search-based multi-cursor select (`*`/`s`), jump list, surround (`ms`)

### M5 — Scripting foundation + polish (complete)
- [x] Whitespace rendering + tab-stop expansion, soft wrap, visual-line `j`/`k`
- [x] Keyboard macros (register-based, `Q`/`q` UX), mouse support (click/drag/scroll)
- [x] Steel scripting engine: `bind-key!`, `define-command!`, `set-option!`, `#:inline-output`, `#:repeatable`
- [x] Configuration via Steel, configurable status line, helix-surround plugin, runaway-script watchdog

### M6 — Plugin infrastructure (complete)
- [x] Message log (`:messages`), PLUM plugin manager (`core:plum`)
- [x] Plugin loading pipeline (attribution, namespace isolation, `load-plugin`/`declare-plugin`)
- [x] Lazy plugin loading (manifest, `#:commands`/`#:events`/`#:languages`, `:plugin-status`)

### M7 — Daily usability (complete)
- [x] Multi-buffer model (`BufferStore`, `:e`/`:bnext`/`:bprev`/`:bd`, hook system)
- [x] Tab completion in minibuffer (command/path/buffer completers, popup UI)
- [x] Command/search history (session-only), `%`/`#` expansion, alternate buffer (`Ctrl+6`)
- [x] `:w!` force-write, statusline line-ending + pwd, `:cd`, `:ls`, `:b` buffer picker
- [x] System clipboard register `'c'` (arboard), smart-p paste with kill ring (`[`/`]` cycling)
- [x] Multiple file paths at startup, tab handling (hard/soft, auto-indent, dedent)

### M8 — Theming (complete)
- [x] Helix theme editor imported and fixed
- [x] Hierarchical theming (Helix-compatible scopes, TOML format, `inherits`, bundled themes, `:theme`/`:theme-debug`)
- [x] `ui.menu`/`ui.menu.selected` scopes for completion popup styling; default theme from `dark.toml`

### M9 — Syntax awareness (complete)
- [x] Language identity (`Buffer.language`, `LanguageRegistry`, `detect_language`, `on-language-set` hook, `define-language!`)
- [x] Syntax highlighting via tree-sitter (grammar loading, `BufferSyntax`, `TreeSitterHighlighter`, `register-grammar!`)
- [x] Grammar installation via PLUM (`plum/ensure-grammars!`, `:plum-install-grammar`, compile step)
- [x] Off-main-thread tree-sitter parsing (parse worker thread, non-blocking request/drain cycle)
- [x] Incremental tree-sitter parsing (`ChangeSet` → `InputEdit`, pending-edit chain, COW tree clone)

### M10 — Splits (planned)

- **Splits & pane focus (`:split` / `:vsplit` / pane focus commands)**: DONE — see T1–T8 below. `LayoutTree::Split` mutation, per-pane render, directional and next-pane focus, close-pane, seam dividers with focus dimming, and per-pane `wrap_mode` (moved onto the engine `Pane`, `hume-engine/src/pane.rs`, as the SSOT — see T6) are all wired.
  - **Sub-tasks**:
    - [x] **T1 — `LayoutTree` mutation primitives** (`hume-engine/src/pipeline.rs`): first add `#[derive(Clone, Debug, PartialEq)]` to `LayoutTree` (currently undecorated) — `remove_leaf` collapses a parent by `mem::replace`-ing the sibling subtree out, and the unit tests below need `assert_eq!`. Then `split_leaf(target, new_pane, direction, ratio) -> bool` (replace `Leaf(target)` with `Split { direction, ratio, (Leaf(target), Leaf(new_pane)) }`); `remove_leaf(target) -> Option<PaneId>` (prune leaf, collapse parent `Split` onto sibling, return sibling id; `None` if sole leaf). Unit tests: split on root, split on nested target, remove collapses parent, remove sole leaf → `None`.
    - [x] **T2 — `:split`/`:vsplit` bodies** (`hume-editor/src/editor/commands/typed_misc.rs`): wrap `Direction`-vs-Vim mapping in a documented helper (`:split` → `Direction::Vertical`, `:vsplit` → `Direction::Horizontal`). `open_pane(focused_buffer_id())` → `view.layout.split_leaf(focused_pane_id, new_pid, dir, 0.5)`. Optional `<path>` arg opens that buffer then splits. Normal-mode precondition. Tests: both panes view same buffer after `:split`; layout is a `Split`; `:vsplit <path>` opens given buffer.
    - [x] **T3 — Multi-pane render / `prepare_frame`** (`hume-editor/src/editor/lifecycle.rs`): the rope and pane-settings closures built in `render_into` currently resolve only the focused pane (all other panes get `PaneRenderSettings::default()`/`None`) — make them resolve every pane's `buffer_id`/settings; `prepare_frame` scroll-sync iterates all panes from `collect_rects_into`. Engine insta snapshot of a `Split` rendering both panes at correct rects.
    - [x] **T4 — Directional pane-focus** (`hume-editor/src/editor/commands/jump.rs`): prerequisite — the pane-focus command signature `(state, view, count, mode)` has no terminal geometry, and no rect cache exists today (the area only lives transiently inside `render_into`/`prepare_frame`); cache the last-rendered `Vec<(PaneId, Rect)>` on `EngineView` during `prepare_frame` so these commands have something to read. Then `cmd_pane_focus_{left,right,up,down}` (currently stubs sharing a `":split not yet implemented"` message) via that cache + nearest-rect-edge from focused pane; `cmd_pane_focus_next` cycles by tree DFS order. Tests: 2×2 grid; directional focus lands on expected pane; `next` cycles all four.
    - [x] **T5 — Close-pane command** (inverted from the original spec — decided during planning): new native `pane-close` (`hume-editor/src/editor/commands/jump.rs`), bound `Ctrl+p c`. `:q` (`typed_quit`) becomes pane-aware instead of gaining a sibling `:close`/`:clo`: if `view.panes.len() > 1` it closes the focused pane (`close_focused_pane` in `commands/mod.rs` — `LayoutTree::remove_leaf(focused)` → refocus returned sibling → drop the four per-pane maps), skipping the dirty check (buffer stays open in the buffer list). Otherwise `:q` falls through to its existing single-pane buffer/quit logic unchanged. `pane-close` on the sole pane warns `"cannot close last pane"` and no-ops — `:q` alone owns quitting. Superseded the dead `#[cfg(test)]` `Editor::close_pane` (mid-cleanup only, no layout prune). Tests: close removes pane + collapses layout + refocuses sibling; `:q` with 2 panes closes pane not editor; `:q` with 1 pane unchanged; `pane-close` on sole pane warns; 2×2 grid close promotes correct sibling.
    - [x] **T6 — Move `wrap_mode` onto `Pane`** (`hume-engine/src/pane.rs`, `hume-engine/src/pipeline.rs`, `hume-editor/src/editor/lifecycle.rs`): added `pub wrap_mode: WrapMode` to the engine `Pane`, seeded from `EditorSettings::wrap_mode` at every pane-construction site (`Editor::new` bootstrap, `commands::open_pane`, `Editor::for_testing`). `PaneRenderSettings` keeps its `wrap_mode` field (renamed/rewritten doc comment) — `resolve_pane_settings` now sources it from `pane.wrap_mode` instead of `doc.overrides.wrap_mode(&settings)`, so the engine pipeline itself is unchanged. Added `Editor::focused_wrap_mode()` SSOT helper; repointed all read sites (`lifecycle.rs` scroll-sync, `mouse.rs`, `commands/mod.rs::focused_format_context`) and the `:toggle-soft-wrap` write site through it. Moved the `wrap-mode` `:set` key from `buffer{}` to `global{}` in the settings macro — **removed** `BufferOverrides.wrap_mode` (kept `tab_width`/`whitespace`); `:set buffer wrap-mode=…` now rejects as global-only. Migrated affected settings/behavior tests; added coverage for per-pane seeding and toggle isolation (two panes on one buffer wrap independently).
    - [x] **T7 — Keymap + docs**: split/vsplit — `build_pane_trie` binds `s` → `pane-split`, `v` → `pane-vsplit` (native `EditorCmd`s sharing `split_pane_onto` with the typed `:split`/`:vsplit`, added in T8); `pane-focus-next` moved from `w` to `p` (frees `w` from any pane-trie ambiguity) — `h`/`j`/`k`/`l` hold directional focus movement, `p` cycles next. Kitty tests updated to real-behavior (`ctrl_p_p_is_noop_with_single_pane`, `ctrl_p_s_splits_pane`, `ctrl_p_v_vsplits_pane`); `c` → `pane-close` bound in T5. End-user docs (`user-manual/docs/commands.md`, `user-manual/docs/key-reference.md`, `user-manual/docs/files-and-buffers.md`) updated with the missing `Ctrl+p c` chord, pane-aware `:q` wording, and a "Splits and panes" concept section; `user-manual/docs/roadmap.md` already listed splits under Implemented. Fixed backwards typed `:split`/`:vsplit` description strings (registry) that had the stacking/side-by-side wording swapped.
    - [x] **T8 — Pane seam dividers + focus dimming** (`hume-engine/src/pipeline.rs`, `hume-engine/src/render.rs`, `hume-editor/src/editor/commands/typed_misc.rs`): `split_rect` reserves a 1-cell seam between sibling panes (`LayoutTree::collect_seams_into`), drawn `│`/`─`, muted by default; the sub-segment adjacent to the focused pane is accent-colored (`focused_seam_segment` — an intersection, not a whole-seam bool, so a seam shared by several sibling panes only lights up beside/above the focused one). Non-focused panes are true-color-dimmed toward `ui.background` (blended inline through `render::PaneCanvas`, not `Modifier::DIM` — inconsistent terminal support). `:split`/`:vsplit` reject with a status warning (no layout mutation) when the focused pane is too small to fit two panes plus the seam (`fits_split`, `MIN_PANE_WIDTH`/`MIN_PANE_HEIGHT`). New `pane-dividers` setting (default on). Added `ui.background`/`ui.window`/`ui.window.focused` to `dark.toml` (`ember.toml` already had `ui.background`). Statusline stays a single global bar — no per-pane statusline. Done ahead of T5–T8 (independent of close-pane/wrap_mode/keymap/lint work).
  - **Implementation order**: T1 → T2 → T3 → T4 → (parity snapshot) → T5 → T6 → T7 → T8. T6 late so multi-pane wrap is a real testbed. T8 done out of sequence, directly after T4 (only depends on the T4 rect cache); T7's split/vsplit keybind + doc portion done alongside T8, `pane-close` binding done directly in T5 (not deferred to T7 after all).
- **Tabline UI**: buffer/tab bar rendered by the engine; `TabBarProvider` slot already exists.

### M11 — Syntax depth (planned)

- **Multi-layer tree-sitter injections**: embedded languages (JavaScript in HTML, code blocks in Markdown). Defer until injection orchestration can be built on the worker architecture already in place.
- **Tree-sitter structural features**: text objects (`locals.scm`, `textobjects.scm`), scope-aware local rename (LSP fallback).
- **`(set-buffer-option! key value)` Steel builtin**: per-buffer option overrides (e.g. `tab-width` per filetype).
  Required pieces:
  1. `active_overrides: BufferOverrides` on `SteelCtx<'a>` — builtin writes there; `set-option!` continues to write to `EditorSettings`.
  2. Persistent `BufferOverrides` slot on `Buffer` merged at render/edit time (currently computed fresh each call).
  3. `set-buffer-option!` builtin in `hume-editor/src/scripting/builtins/settings.rs` — valid only during `call_steel_cmd`; error if called at init time.
- **Code folding** (tree-sitter powered collapse/expand)

### M12 — Editor chrome (planned)

- **Class B chrome slot (bottom drawer)**: full-width, auto-sized, capped at ~50% terminal height. Hosts transient read-only content: `:ls`, `:messages`, LSP hover docs, notifications, command/search history pagers. Not a Pane — spans full terminal regardless of split layout. Editor-side viewport sync queries engine chrome height instead of hardcoding `-1` for statusline only.
- **File picker / fuzzy finder** (Helix-style): depends on split/pane layout. Deferred until post-splits.
- **Class A docked panes (fixed-row-count `LayoutTree` variant)**: real panes docked to a fixed row count inside the split tree. `LayoutTree::Fixed { rows, main, dock }` alongside `Split { ratio }`. Clients: quickfix list, LSP references/diagnostics, embedded terminal/REPL, build/test runner, `:help` pager, DAP debugger views. Deferred until the first concrete client is scoped.

### Future

- **Wrap indicator**: configurable char prepended to continuation rows in soft-wrap mode.
- **Steel: `on-buffer-switch` hook + per-buffer keymaps**.
- **PLUM: pin plugins to commit / tag / branch**: `(declare-plugin "user/repo" #:rev "v0.3.1")` syntax.
- **`:e <new-path>` touch-or-open**: create empty buffer bound to path when file doesn't exist; first `:w` writes the file.
- **`:e` binary / huge-file y/n confirm**: binary-sniff + size threshold → `Mode::Confirm` with callback storage.
- **Byte-string parsing in settings**: `"10MB"` / `"512KB"` strings; companion to size-threshold setting.
- **Cached `size: u64` on `FileMeta` + `FileSize` statusline element**.
- **Streaming load for huge files**: chunked `Rope::from_reader` replacing single blocking `fs::read_to_string`.
- **LSP support** (Rust transport + Steel behavior layer): completions, diagnostics, hover, go-to-definition, rename.
- **Virtual lines / decoration layer** (inline diagnostics, ghost text, code lenses, inlay hints): depends on LSP.
- **Unified decoration system**: single `Decoration` trait replacing the current separate provider traits (`GutterColumn`, `HighlightSource`, `VirtualLineSource`, `InlineDecoration`, `OverlayProvider`). Post-LSP, once the decoration surface is stable.
- **Steel builtin to register custom completers**: plugin-side `Completer` implementations dispatched by command name; core does prefix matching only (fuzzy scoring is a plugin concern).
- Git gutter signs (plugin candidate — keep out of core)
- File watcher (detect external file changes, prompt to reload)
- Documentation: Markdown guides, auto-generated command reference, in-editor `:help`
