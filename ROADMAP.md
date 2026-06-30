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
| Line selection | **`x`/`X` (walk down/up lines)** | `x` selects current line including `\n`; repeated `x` walks to next. `Ctrl+x`/`Ctrl+X` accumulate lines (force_extend, works in both kitty and legacy). `o` in extend mode flips anchor/head. |
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
| Disjoint-borrow refactor | **Free functions, not `&mut self` facility methods** | `Editor` is a flat state container. `&mut self` facility methods borrow the whole struct, forcing borrow-workaround clones. Free functions taking only the fields they need let Rust prove borrows are disjoint. Rule enforced by a lint in the build. |
| Kitty push order on init | **Push after entering alt screen** | Some terminals maintain a per-screen keyboard stack. Pushing before `EnterAlternateScreen` lands flags on the primary screen's stack, which key reads don't consult. |
| SHIFT normalization | **Strip bare SHIFT for any Char** | Terminals that don't honour `REPORT_ALTERNATE_KEYS` send shifted punctuation as `Char + SHIFT`, missing trie bindings. Strip SHIFT for any `Char` when SHIFT is the only modifier (`Ctrl+Shift` keeps CONTROL; `Shift+Tab` = `BackTab` is not a `Char`). |
| Kill-ring whitespace collapse | **Overwrite pure-whitespace head in place** | On push, if the ring head is pure-whitespace, the new entry overwrites it instead of taking a fresh slot. Keeps swap-junk from filling the ring; the whitespace stays retrievable until the next push. |
| Tab handling | **One knob (`tab-style`), reuse `tab-width`** | Tab key inserts `\t` (hard) or spaces to next tab stop (soft). `tab-width` reused for both rendering and Tab-key spacing. Vim's four-knob `shiftwidth`/`softtabstop` model rejected as too complex. Enter copies leading whitespace verbatim. |
| Diff module | **histogram + Myers fallback, `similar` hidden** | `diff_lines`: histogram with wall-clock deadline, falls back to Myers on timeout. `diff_words`: Myers over UAX #29 word-boundary tokens with deadline. `similar` is a hidden impl detail — public types owned by `hume-editing` so consumers never depend on the backend. |

## Open Questions

| Question | Context |
|----------|---------|
| Multiline quote text objects | Quote text objects (`i"`, `i'`, `` i` ``) are line-bounded because the parity scan gives wrong results when earlier lines contain unmatched quotes. Brackets don't have this problem (asymmetric delimiters allow depth tracking). Tree-sitter can resolve the ambiguity — use syntax-aware matching when a grammar is loaded, fall back to line-bounded parity otherwise. |
| Register paste count mismatch | When yank uses N cursors but paste uses M≠N, Helix falls back to pasting the full register at every cursor. Explore alternatives with real usage data (e.g. cycling slots, clamping to last slot, user-facing warning). Decide after more real usage. |
| Common-prefix auto-extend on Tab | Readline/bash behaviour: on Tab with ≥2 candidates, first extend the input to their shared prefix before opening the popup. Current behaviour skips straight to cycling. Small UX polish; decide after more real usage whether the extra keystroke saved is worth the surprise of input mutating mid-type. |
| Plugin-defined languages × lazy loading | Language identity/detection is HUME-owned (`runtime/scheme/languages.scm`, eager). A plugin keying `#:languages` off an *existing* language works fine. But a plugin that defines its **own** language must register that identity **eagerly** — a lazy body cannot be the sole provider of its own activation language (chicken-and-egg). Deferred to a dedicated brainstorm. |
| `llvm-mir` / `llvm-mir-yaml` grammar mismatch | Inherited from Helix: `llvm-mir-yaml` defines the `.mir` extension but delegates to the `llvm-mir` grammar (no separate compiled grammar). `llvm-mir` itself has no extension binding. Track upstream; no HUME-specific fix needed until then. |

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

### M9 — Syntax awareness (in progress)
- [x] Language identity (`Buffer.language`, `LanguageRegistry`, `detect_language`, `on-language-set` hook, `define-language!`)
- [x] Syntax highlighting via tree-sitter (grammar loading, `BufferSyntax`, `TreeSitterHighlighter`, `register-grammar!`)
- [x] Grammar installation via PLUM (`plum/ensure-grammars!`, `:plum-install-grammar`, compile step)
- [x] Off-main-thread tree-sitter parsing (parse worker thread, non-blocking request/drain cycle)
- [x] Incremental tree-sitter parsing (`ChangeSet` → `InputEdit`, pending-edit chain, COW tree clone)
- [ ] **Multi-layer tree-sitter injections**: embedded languages (JavaScript in HTML, code blocks in Markdown). Defer until injection orchestration can be built on the worker architecture already in place.
- [ ] **Tree-sitter structural features**: text objects (`locals.scm`, `textobjects.scm`), scope-aware local rename (LSP fallback).
- [ ] **`(set-buffer-option! key value)` Steel builtin**: per-buffer option overrides (e.g. `tab-width` per filetype).
  Required pieces:
  1. `active_overrides: BufferOverrides` on `SteelCtx<'a>` — builtin writes there; `set-option!` continues to write to `EditorSettings`.
  2. Persistent `BufferOverrides` slot on `Buffer` merged at render/edit time (currently computed fresh each call).
  3. `set-buffer-option!` builtin in `hume-editor/src/scripting/builtins/settings.rs` — valid only during `call_steel_cmd`; error if called at init time.
- [ ] **Code folding** (tree-sitter powered collapse/expand)

### M10 — Splits (planned)

- **Splits & pane focus (`:split` / `:vsplit` / pane focus commands)**: stubs already registered; multi-pane scaffolding on `Editor` is split-ready. Requires a layout engine to render side-by-side views.
  - **Design note — move `wrap_mode` onto `Pane` when wiring splits**: wrap is a view property; two panes sharing a buffer at different widths each want their own wrap mode. Add `pub wrap_mode: WrapMode` to `Pane`; initialise from `EditorSettings::wrap_mode`; read from `pane_ctx.pane.wrap_mode` in the format pipeline. Keep `tab_width` and `whitespace` on `Buffer.overrides` — they are document preferences, not view preferences.
- **Class B chrome slot (bottom drawer)**: full-width, auto-sized, capped at ~50% terminal height. Hosts transient read-only content: `:ls`, `:messages`, LSP hover docs, notifications, command/search history pagers. Not a Pane — spans full terminal regardless of split layout. Editor-side viewport sync queries engine chrome height instead of hardcoding `-1` for statusline only.
- **Tabline UI**: buffer/tab bar rendered by the engine; `TabBarProvider` slot already exists.
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
