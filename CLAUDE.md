# HUME — Project Instructions

## What is this?
HUME (HUME's Unfinished Modal Editor) is a modal text editor for the terminal, written in Rust. This is an agentic programming / learning project.

## Key files
- `README.md` — Project description
- `docs/CRATES.md` — Workspace crates, what each owns, dependency edges
- `docs/ROADMAP.md` — Open questions and milestones
- `docs/LSP.md` — LSP design, architecture, and decisions
- `docs/LEARNING.md` — Concepts and Rust patterns explained as they arise

## Quick orientation
- **Named commands** (`hume-ops/src/edit/`, `hume-ops/src/motion/`) are pure functions of buffer + selections (plus command-specific params like `count: usize`, `MotionMode`). Edits also return a `ChangeSet`. They have no knowledge of keys — `hume-ops` doesn't depend on `hume-editor`, so this is compiler-enforced, not just discipline.
- **Keymaps** (`hume-editor/src/editor/keymap/`) map `KeyEvent` sequences to command names via a trie. Per-mode keymaps (Normal, Extend, Insert).
- **Buffer invariant**: every buffer always ends with a structural `\n`. Cursors always satisfy `head < len_chars()`.

## Rules
- **Record the *why* of a decision in a source comment** at the implementing site, not in ROADMAP — comments stay next to the code they explain instead of drifting into a second, unmaintained copy of it. Update `docs/ROADMAP.md` only to remove a resolved open question or when milestones change.
- **Rust idioms**: Write idiomatic Rust. Prefer pattern matching, iterators, and the type system over runtime checks. Use `Result` and `Option` — no `.unwrap()` in non-test code.
- **Terminal compatibility**: Require true color (24-bit) and synchronized output. Prefer kitty keyboard protocol but fall back gracefully to legacy encoding when unavailable. No shims for truly ancient terminals.
- **Cross-platform**: macOS primary, Linux and Windows (Git Bash / WSL) secondary. Use `termina` or similar abstractions for platform differences — no platform-specific code unless behind `cfg` gates.
- **Keep it simple**: This is a learning project. Prefer clarity over cleverness, and direct solutions over premature abstraction.
- **Testing**: Every editing command, text object, and selection operation must be tested. No untested commands. Core editing logic uses state triples (`initial, op, expected` with cursor/selection markers). Appearance (glyphs, spacing, exact rendered strings) is tested with `insta` snapshots — inline for short one-line element strings, file snapshots for full-frame renders — never with hardcoded string assertions in unit tests; unit/integration tests assert data/semantics only. Verification sequence: `cargo fmt`, then run this exact command line from the repo root, exactly once, before pushing:
```
scripts/test-all.sh
```
`test-all.sh` matches CI exactly: it fetches the grammar fixtures the suite requires and runs doctests, so it supersedes a bare `cargo test` — never run a full `cargo test` before it, and never rerun the full suite after `fmt` (whitespace-only, doesn't invalidate a green run). **A second full run is forbidden** — this includes re-invoking `scripts/test-all.sh` piped through `tail`/`grep`/`tee` or any other filter "to double-check" a result already reported; that still executes the whole suite again. If the first run's outcome is unclear (truncated output, ambiguous exit), treat it as a successful run — do not re-run to settle the doubt. Narrow `cargo test <filter>` runs while iterating on a specific failure are fine.
- **Editing model**: Select-then-act. Keys bind to named commands, not to other key sequences. No key-to-key remapping.
- **Scripting**: Steel (Scheme) for plugins and configuration. Rust handles performance-critical paths; Steel handles behavior and customization.
- **`.scm` files carry a banner only, never prose**: `hume-scripting/src/builtins/bootstrap.scm` and `print_gate_shims.scm` are `include_str!`'d verbatim into a `&str` const and parsed by Steel — their explanatory prose lives in the Rust comment block above that const in `builtins/mod.rs`, never in the `.scm` itself. `runtime/scheme/*.scm` follows the same shape for a different reason: it ships to every user's disk (`scripts/stage.sh` copies `runtime/` wholesale), and its prose lives in that directory's `README.md` (`prelude.scm`'s in `prelude.md`) instead. Both are carve-outs from "record the *why* at the implementing site" — a `.scm` file's nearest implementing site is a Rust doc comment or a sibling README, never itself. Prose that already has a home elsewhere (a builtin's own Rust doc, `docs/LSP-INSTALL.md`'s design rationale) stays there and is not duplicated into the README either.
- **"engine" is overloaded — disambiguate**: Bare "engine" always means the `hume-engine/` crate. The Steel VM is "Steel engine" in prose and `steel` (or `SteelEngine`) as field/type — never a bare `engine` field. Applies to comments and docs alike.

## Day-one invariants

These must be respected from the first line of code — retrofitting them later is expensive.

Most are enforced by a family of domain types (`CharOffset`, `RopeyLine`/`ContentLine`, the five column types) with a private field and no `Add`/`Sub`/`AddAssign` impl: a raw `x + 1` doesn't type-check, and a function typed for one domain can't be handed another's value. Each entry below states only its own mints, arithmetic, and exceptions, not that boilerplate again. `.index()`/`.get()` is each type's own escape hatch into a foreign coordinate system (ropey, tree-sitter, LSP wire positions) with no domain of its own; a terminal/pane cell coordinate uses the `x`-family (`screen_x`/`content_x`/`pane_x`) instead of a column type.

See `CONTRIBUTING.md`'s "strongest tool that fits" bullet for why some invariants stop at the type system, some are a `clippy::disallowed_methods` entry instead (workspace-wide, enforced by `cargo clippy --workspace --all-targets -- -D clippy::disallowed_methods` via `scripts/test-all.sh`/CI — CI runs `ubuntu-latest` only, so a banned call behind `#[cfg(windows)]` is invisible to this gate), and some are an `arch-lints/` scanner.

### Selections

Selections live in a `SelectionSet` (`hume-editing/src/selection/mod.rs`) — `Vec<Selection>` plus a `primary: usize` index, kept sorted by start, non-overlapping, non-empty. All edit operations iterate over selections. Selections are always inclusive: `anchor == head` is a 1-char selection covering the character at that index, never a zero-width point.

### Grapheme clusters

All motions, selections, and edit operations work on grapheme clusters (`unicode-segmentation`), never raw bytes or `char` — this is the text boundary abstraction.

| | |
|---|---|
| **Forbidden** | Stepping a buffer position by a raw `+ 1`/`- 1` in motion or selection code — skips over combining sequences (`é` = U+0065 + U+0301) or ZWJ emoji instead of advancing a full cluster. |
| **Required** | `next_grapheme_boundary`/`prev_grapheme_boundary` (`hume-editing/src/grapheme.rs`, thin `&BufferText` wrappers over the `RopeSlice`-based implementations in `hume-rope/src/grapheme.rs`) for every position advance in motion/selection logic. |
| **Allowed** | `line += 1` for line-level iteration; `i += 1` in bracket/delimiter scanning (ASCII only). |
| **Enforced** | The compiler, via `CharOffset` — see that entry below. One hazard no type reaches: `s.chars().next_back()`/`.last()` return `s`'s last *codepoint*, not the base char of its last cluster (a trailing combining mark) — `hume_rope::grapheme::prev_str_boundary` is the fix. No automated check for this one; relies on ordinary review. |

### Buffer char offsets

Every buffer position — `Selection::anchor`/`head`, motion and text-object results, `ChangeSetBuilder`'s running cursor — is a `hume_rope::offset::CharOffset` (`hume-rope/src/offset.rs`), never a bare `usize`.

**Mints** — `::new` (trusted, no rope check: the value is already known valid, e.g. read back from another `CharOffset`, or the result of an ASCII-only scan), `::checked` (validates against a `&Rope`, `None` past `len_chars()` — Steel builtin args), `Default` (offset 0). No `::clamped`/`::snapped`: a wire position clamps line-then-column (`hume_rope::position_encoding::wire_to_char`), and landing on a grapheme boundary is `hume_rope::grapheme::snap_to_cluster_start`'s job directly — both more specific than a blind `idx.min(rope.len_chars())` on an already-known `CharOffset` would be.

**Arithmetic** — `.index()` escapes into ropey, tree-sitter, or any other foreign coordinate system. `chars_since` gives a char count between two offsets (debug-asserts on inversion, not a `Sub` impl — a raw subtraction would silently wrap). `shift(delta: isize)` repositions by a signed delta already known to preserve grapheme alignment. `retreat(n: usize)` is `shift`'s unsigned-length counterpart, for the common case of retreating by a `usize` count (a removed run's length, a typed-char count) rather than a signed delta a caller would otherwise negate by hand. `retreat_saturating(n: usize)` is `retreat` clamped to 0 instead of panicking, for a count that may legitimately exceed `self` (a cramped cursor near the buffer start, an edit delta larger than the position it lands on).

**Ranges** — `ExclusiveRange<CharOffset>`/`InclusiveRange<CharOffset>` (same module) name the two range conventions the codebase mixes: half-open (`ChangeSet`'s position-mapping API, viewport/diagnostic ranges, `BufferText::slice`) vs. both-ends-covered (`Selection`, every text-object/bracket/quote/tag/search finder) — instead of a bare `(usize, usize)` meaning either. Both mint via `::new` plus field access plus `contains`. `ExclusiveRange` alone also has `is_empty` (any `T: PartialEq`, not `CharOffset`-specific — `hume-engine`'s per-grapheme `ExclusiveRange<ByteCol>` is a caller too); `InclusiveRange` has none, since it is never empty by construction. `InclusiveRange<CharOffset>` alone adds `to_exclusive` (`self.end.shift(1)`, not a raw `.index() + 1`), returning an `ExclusiveRange<CharOffset>` — the inclusive→exclusive crossing an inclusive result (`Selection`, a finder) needs before it can reach `BufferText::slice`, which takes `ExclusiveRange<CharOffset>` directly rather than a bare `Range<usize>`. `Selection::end_exclusive`/`content_end_exclusive` give that same bound without hand `+ 1` arithmetic.

**Enforced** — the compiler (see preamble). Two places a buffer position stays plain `usize`: the Steel/LSP host boundary (`hume_scripting::host::EditorHost` and its per-capability traits, `hume-editor/src/editor/host_impl/`) — a documented FFI contract (`docs/LSP.md`) that Steel decodes as a raw integer regardless, converting to/from `CharOffset` immediately at that seam — and `hume_engine::types::Grapheme.char_offset`, which needs a `usize::MAX` sentinel for a virtual display line's cells (no buffer position at all) that `CharOffset` has no room to represent; the comparison sites in `hume-engine/src/display_lines/locate.rs` and the mint in `display_lines/render.rs` document the trade-off.

### Line counts and ranges

Every "how many lines" / "which line is last" / "range of lines" computation goes through `hume-rope`'s four functions — `ropey_line_count`, `last_ropey_line`, `content_line_count`, `last_content_line` — or a `hume_editing::text::BufferText` method that delegates to one, never a raw `len_lines()` call. Production never walks a whole buffer by line index (the render path walks a viewport window, motions scan outward from the cursor), so there is no whole-buffer *range* function alongside these four; a test needing to walk every line writes the one-line `(0..count.get()).map(Type::new)` inline.

`hume-rope` distinguishes *ropey domain* (the phantom trailing line the buffer invariant's structural `\n` creates, included) from *content domain* (that phantom line excluded) — conflating the two is an off-by-one. The distinction is compile-time: `RopeyLine`, `ContentLine`, `RopeyLineCount`, `ContentLineCount` (`hume-rope/src/line.rs`) wrap the two domains in distinct types, so a raw `line + 1`/`count - 1` on one of the four functions' own result doesn't type-check.

**Mints** — `RopeyLine`: `::new` (trusted), `::clamped` (clamps against a `&Rope`) — no `::checked`, since every ropey-domain index up to and including the phantom trailing line is valid by definition — and widens from `ContentLine` via `From` (a content-domain line is always also a valid ropey-domain one). `ContentLine`: `::new`, `::checked` (validates, `None` past the last real line), `::clamped`, and `::from_number` (a 1-based line number — a Steel builtin argument, a `:goto` target) with `.number()` as its inverse (the 1-based read-back, same pairing as `GraphemeCol::from_number`/`number()`). Only `ContentLine` derives `Default` (line 0) — load-bearing via `DisplayLinePos`/`ScrollPosition`; `RopeyLine`'s would have no caller, so it doesn't. A `RopeyLine` narrows to a `ContentLine` via `.to_content(rope)` (`None` on the phantom line) — production narrows with the trusted `ContentLine::new(line.index())` exactly where the caller already proved the value can't be the phantom line, reserving the validating forms for input whose domain isn't yet established.

**Arithmetic** — `advance(n)` (both types — unclamped, matching `CharOffset::shift`/`retreat`'s own bare names for "no saturation": there is no ropey-domain ceiling either type can check without a `&Rope` in hand) and `retreat_saturating(n)` (`ContentLine` only — a ropey-domain line has no saturating-backward caller; named to match the same saturate-at-0 contract's name on `CharOffset`/`BufferLineCol`, not `up`, since this codebase never lets a bare direction word and a `_saturating` suffix name the same behavior) are a *named* escape from the no-arithmetic rule, not a hole in it: a re-derivation like `last_content_line(rope).advance(1)` compiles and is the sanctioned way vertical-motion code steps a line. A bare-`usize` loop bounded by two typed endpoints' own `.index()`, re-minting `ContentLine::new` each iteration rather than stepping the endpoint with `.advance(1)` (`hume-ops`'s `edit::join`, `edit::sort`, `edit::indent`, `selection_cmd::matching`), is unenforced the same way — `ContentLine::new` being `pub` admits it. Each of the four carries its own in-file rationale comment at the loop itself. `line_break_char(rope, line)` replaces a raw `next_line_start(rope, line) - 1` for the one pattern the types can't reach (both sides are already-`usize` char offsets after `.index()`) — named for what it is, so the subtraction reads as the mistake it would be. `ContentLine` also has `abs_diff` (unsigned distance, direction discarded — a jump-distance threshold) and `lines_since` (forward-only, debug-asserts on inversion, mirroring `CharOffset::chars_since`); `ContentLineCount::end_exclusive` mints the `ContentLine` one past the last real line — the exclusive upper bound a half-open content-domain range ends at, not a raw `ContentLine::new(count.get())`.

**Carve-outs** — `hume-ops/src/motion/paragraph.rs` scans bare `usize` content-domain line indices internally, and `hume-editor/src/editor/visual_move.rs`'s vertical-motion math scans bare `isize` ones — both with their own in-file rationale.

**Enforced** — the compiler (see preamble). `clippy.toml`'s `disallowed-methods` bans `ropey::Rope`/`RopeSlice`'s own `len_lines`/`line_to_char`/`char_to_line` workspace-wide; `hume-rope/src/lines.rs`'s `ropey_line_count` and its sibling line-start/line-count functions (`line_start_char` for `&Rope`, `slice_line_start_char` for a `RopeSlice`) are the sanctioned wrappers, the file carrying one `#[allow(clippy::disallowed_methods)]` for all of them. `hume-rope/src/grapheme.rs`'s three line-relative column resolvers (`grapheme_col_in_line`, `display_col_in_line`, `char_pos_at_display_col`) route through `slice_line_start_char` rather than narrowing a `ContentLine` back to `RopeSlice::line_to_char` directly, so that file carries no `#[allow]` of its own.

**FFI seam** — unlike `CharOffset` (which converts to/from raw `usize` at `host_impl.rs`, per that section's own "Enforced" note), a line-domain value crossing the Steel/LSP `Host` trait boundary stays typed (`ContentLine`, `ExclusiveRange<ContentLine>`) all the way through the trait signature and `host_impl.rs`'s implementation — it is unwrapped to a raw integer only in `hume-scripting/src/builtins/*.rs`, at the layer that already bounds-checks the value against `buffer_line_count`/`content_line_count` before minting it. This is a deliberate difference in *where* the two seams convert, not a drift: `CharOffset`'s host-side callers rarely need the type for anything past the call, while the line-domain builtins' bounds check is what licenses the trusted mint, so keeping the type until that check has run (rather than converting one layer earlier, at `host_impl.rs`) keeps the mint beside its own justification.

### Display columns

Every "how many terminal columns does this text occupy" computation goes through `hume_rope::width` (`tab_advance`, `grapheme_width`, `str_width`) — never a direct `unicode-width` call. Grapheme *indexing* (`hume-rope/src/grapheme.rs`) counts clusters, not cells; LSP wire positions count UTF-16 code units or bytes — neither is a display column. `hume_rope::width` itself stays primitive `usize`/`u32`, not domain-typed like "Line/buffer columns" below: it is the raw measurement layer (ropey's analogue in the `CharOffset` model), called with display-line-relative columns (`hume-engine`'s formatter), buffer-line-relative ones (`hume-ops`'s tab/indent math), and run-relative terminal-cell offsets (`hume-grid`'s `Canvas`) alike, so it cannot commit to one origin. Callers narrow with a domain type's `.get()` (see preamble).

**Drawing** — write through `Canvas` (`write_text_run`/`fill_glyph_run`/`write_cell`/`fill_rect_bg`/`fill_row_bg`, `hume_grid::Canvas`, re-exported as `hume_engine::render::Canvas`), never `hume-grid`'s own `Grid::set_glyph`/`fill_span` — those are `pub(crate)` to `hume-grid`, so a call from anywhere else is a compile error. `write_text_run` is the one to read: it walks by `grapheme_width`, so measurement and drawing can't drift; it takes a `right_edge` so a field sized with `str_width` can't bleed through a pane, lane, or box border; and it explains why an unrenderable or control cluster draws as its codepoint placeholder rather than a blank. Full rationale lives on `write_text_run`'s own doc comment (`hume-grid/src/canvas.rs`) — every other mention, this one included, is a pointer, not a second copy.

**Enforced** — `clippy.toml`'s `disallowed-methods` bans `unicode_width`'s own methods workspace-wide; `hume-rope/src/width.rs` itself and three independent-oracle test assertions (`hume-ui`, `hume-editor`) carry `#[allow(clippy::disallowed_methods)]`.

### Line/buffer columns

"Column" means five different things in this codebase:

| Sense | Type / spelling | Where |
|---|---|---|
| Display column, buffer-line origin | `BufferLineCol` | terminal cells, tab-expanded |
| Display column, display-line origin | `DisplayLineCol` | terminal cells, tab-expanded, from a continuation line's own left edge |
| Char column | `CharCol` | char index within a line |
| Grapheme column | `GraphemeCol` | grapheme-cluster index within a line |
| Byte column | `ByteCol` | byte offset within a line |
| *(no type — gutter widget)* | `GutterColumn`/`SignColumn`/`LineNumberColumn` (CamelCase) | names a gutter widget, not a coordinate |

All five typed senses live in `hume_rope::column` (`hume-rope/src/column.rs`), matching `CharOffset`/`RopeyLine`/`ContentLine`'s own convention. A bare `*LineCol` is always display cells; a unit prefix (`Char`/`Grapheme`/`Byte`) names any other unit. LSP wire positions use a bare `character: usize` (the protocol's own term, its own code-unit domain) — never one of these five. A terminal/pane cell coordinate uses the `x`-family (see preamble), never a column type; a lane-bound local variable is named `lane`, never `col`.

**Origin** — under soft wrap, a continuation display line renumbers its columns from its own left edge, so "display column" means two different things depending on whether it's counted from the display line or from the whole buffer line — the same character has a different number in each. With wrapping off the two coincide (`BufferLineCol::as_display_line_unwrapped` is the one sanctioned buffer→display crossing, for exactly that case). Two concrete types, not a single phantom-typed `DisplayCol<Origin>` — the origin is chosen at runtime (`hume-editor`'s vertical motion reads the live wrap mode), so a phantom parameter would only have to be erased right back out. `Selection::sticky_display_col` (`hume-editing/src/selection/single.rs`) is `StickyDisplayCol::DisplayLine { display_col: DisplayLineCol, wrap_width: Option<u16> } | BufferLine { display_col: BufferLineCol }` for the same reason `CharOffset` forbids `+ 1`: reusing a latch tagged with the wrong variant would read a display-line-relative number as a buffer-line-relative one (or vice versa) and land sideways — the enum makes that a compile error at the read site instead of a runtime bug.

**Arithmetic** — unlike `CharOffset`, a display column is a genuine accumulator (formatting advances one grapheme's width at a time), so `DisplayLineCol`/`BufferLineCol` expose named arithmetic instead of forbidding it outright: `advance_saturating`/`cells_since` on both, `abs_diff`/`cells_since_saturating` on `DisplayLineCol` alone, `shift_saturating`/`retreat_saturating` on `BufferLineCol` alone (`retreat_saturating(cells: u32)` is `shift_saturating`'s unsigned-length counterpart, for the common case of retreating by a cell count rather than a signed delta) — every one of them carries the `_saturating` suffix, not a bare `shift`/`retreat`/`advance`, because a display column saturates at its bound instead of panicking on an out-of-range result, the opposite of `CharOffset`'s contract for the same bare names. `advance_saturating` is the per-grapheme workhorse of every format walk, the rest have a small handful of callers apiece; exact counts live at each method's own doc comment in `hume-rope/src/column.rs`, not duplicated here. `CharCol` stays as strict as `CharOffset` (mint via `new`/`index()` only); `GraphemeCol` adds `from_number`/`number()` for its 1-based display form, but no arithmetic. `ByteCol` has one named exception, `advance_saturating` (a byte length folded onto a byte column — a tree-sitter edit's end position, a bracket match's end byte); every other operation on it stays as strict as `CharOffset`. `hume-engine`'s per-grapheme `Grapheme.byte_range` (`hume-engine/src/types.rs`) is an `ExclusiveRange<ByteCol>` — the formatter's per-cell hot path, same as every other `ByteCol` span in the workspace, not a bare `Range<usize>` carve-out. `ExclusiveRange<ByteCol>::as_byte_range()` (`hume-rope/src/column.rs`) gives back the `std::ops::Range<usize>` a `str`/`&[u8]` slice call wants, in one crossing rather than two `.index()` calls at the slice site.

**Enforced** — the compiler (see preamble), plus one gap no type closes: a bare `usize`/`u16` named `col`/`column` in a crate with no column type of its own (`hume-grid`, `hume-platform`, `hume-scripting`, `hume-lsp`, `hume-ui`, `hume-decorations`) is invisible to the compiler, and nothing polices a *new* `FooColumn` CamelCase type actually naming a gutter widget rather than a coordinate. `hume-scripting/src/host/lsp.rs`'s `LocationDisplay.grapheme_col_or_wire: Option<usize>` is the sanctioned display-value exception below — its *name* is the only thing carrying that meaning. `hume-scripting/src/host/edits.rs`'s two bare `char_col: usize` FFI params are the addressing-unit case the same rule permits — a Steel builtin argument, never rendered.

**Displayed value** — every column HUME *shows* a user (statusline, `:diagnostics`, LSP goto/references) is a 1-based `grapheme_col`/`grapheme-col` — the unit the editing model itself counts in. `char_col`/`char-col` and LSP wire `character` are addressing units only (feeding `goto-location!`, decoration setters, protocol requests) and must never be rendered directly. One sanctioned exception: the goto/references drawer's column for a target with no open buffer (`lsp-locations->display-parts`, `hume-editor/src/editor/lsp/introspect.rs`'s `location_display_parts`) renders the location's own wire `character` verbatim rather than reading the file to convert it — the file may never be opened, so counting graphemes in it isn't worth a disk read. The value carrying that union is named `grapheme_col_or_wire`, never bare `grapheme_col`, so its own name doesn't overclaim. Nowhere else may this exception be cited.

### Buffer lines, display lines, and rows

Three words, three meanings, never interchanged.

| Term | Is | Lives in |
|---|---|---|
| **Buffer line** | a `\n`-delimited line of the rope | `RopeyLine`/`ContentLine` (above) |
| **Display line** | one visual line a buffer line occupies once soft wrap, provider-injected virtual lines, and tilde filler are accounted for | `hume-engine`'s `DisplayLine`/`DisplayLineKind`/`DisplayLineMap` (`display_lines.rs`, `types.rs`), `DisplayLinePos` (`display_lines/pos.rs`) — the single flattened view every render/scroll/cursor/mouse/movement consumer walks |
| **Row** | a terminal or pane cell row (the *y* axis) | `screen_row`/`start_screen_row`, `hume_grid::Grid::row`/`RowRun`/`Canvas::fill_row_bg`, and every widget's own rows (`list_rows`/`selected_row`/`styled_rows`) |

The three coincide only by accident (an unwrapped, undecorated line's one display line happens to land on one terminal row) and the types never assume it. A terminal/pane cell row stays a bare `u16`/`usize` named `row`/`*_row` (see preamble's `x`-family) — never a display-line type; a display-line *count* or *address* (`DisplayLineMap::advance_saturating`'s `delta`, `DisplayLinePos`) is never spelled `row`.

**`DisplayLinePos { line, slot }`** addresses one display line: `line` names the buffer line, `slot` its index within that line's own visual block (`before`-virtuals, then the line's own wrap/content display lines, then `after`-virtuals, in order). `Viewport::top` (`hume-engine/src/pane.rs`) is this same pair, as one `DisplayLinePos` field. `slot` is display-line-exclusive vocabulary within `hume-engine`'s viewport/scroll machinery — a same-named but unrelated concept (how many panes share a split) lives in `hume-engine/src/pipeline/layout.rs` as `share`/`shares_along`, not `slot`, specifically so the two never collide under one name in the same crate (see "'engine' is overloaded", above). One carve-out from that same-crate exclusivity: `hume_engine::builtins::sign_column::Sign::slot` is a user-facing term (the gutter sign slot a plugin reserves via `register-sign-source!`) predating and unrelated to the display-line sense.

**Two more carve-outs, both load-bearing.** Tree-sitter's own `Point { row, column }` (`hume-treesitter/src/edits.rs`, `hume_rope::lines::advance_byte_point`) is a foreign coordinate system this codebase decodes, not one it defines — its `row` counts `'\n'`s in a byte range, unrelated to either buffer or display lines, and is never renamed to match. `:sort`'s user-facing vocabulary (the `:sort` error text, `user-manual/docs/command-mode.md`) calls a buffer line a "row" for the end user, predating and unrelated to this distinction — the internal representation is never named "Row".

**Enforced** — none; the only invariant on this page with no automated check. The naming distinction is compiler-invisible (all three are ultimately `usize`/`u16`-backed) and test names, prose, and cross-references have never been in scope for any scanner. Relies on ordinary review — a drifted identifier here compiles clean.

## Rust coding philosophy
This project is both a product and a learning journey. Write the best Rust possible, and teach as you go.
- **Idiomatic first**: Use the type system, iterators, pattern matching, and ownership as intended. Don't fight the borrow checker. Follow current best practices.
- **Performance by design**: Choose the right data structures and algorithms upfront. Avoid allocations in hot paths, use iterators over index loops.
- **No magic**: No macro-heavy abstractions that hide what's happening. Macros only when they genuinely reduce boilerplate.
- **Clean and readable**: Performance and clarity are not at odds in Rust — the compiler optimizes idiomatic patterns well. When in doubt, prefer the version a newcomer can follow.

## Documentation audiences
Every piece of writing in this repo targets one of three audiences. Know which one before you write, and don't mix them.

1. **End users** — people running HUME who want to use it. Lives in `CHANGELOG.md`, `README.md`, `user-manual/docs/*.md`, `runtime/tutor.rst`, `runtime/init.scm.example`, and any `:help`-style content surfaced inside the editor.
   - No internal names. Don't reference Rust types, Steel builtins used only by the implementation, or module paths. ❌ "the next key pressed is passed as `(pending-char)`" — `pending-char` is a code internal; describe the *behaviour* instead.
   - No babysitting. Assume the reader can follow a short instruction. ❌ "These are absent on a fresh setup" — say what to do, not what the reader will or won't see.
   - Describe what the editor does and how to drive it. Nothing about why it's built that way.

2. **Learners / curious developers** — people who want to understand HUME's *concepts*, not its code yet. Lives in `docs/LEARNING.md` and `docs/learning/*.md`.
   - High-level explanations of ideas: the text model, motions vs text objects, the undo tree, etc.
   - No source-file paths, no `editor/src/...` references, no function names. If the explanation needs them to land, it belongs in a source comment instead.
   - Code snippets are fine when they illustrate an *idea*; they should read as pseudocode-with-Rust-syntax, not as a tour of the actual implementation.

3. **Source readers (contributors)** — people with the file open, or one directory away from it. The doc surface for this audience is inline source-code comments, plus a per-directory `README.md` (e.g. `scripts/README.md`, `runtime/scheme/README.md`, each `runtime/plugins/core/*/README.md`) for prose a comment can't carry without either drowning the code it sits in or getting duplicated across every file it'd otherwise need repeating in. Conversely, source-code comments and these READMEs target only this audience — never an end user, never a learner.
   - Add a brief comment when the *why* is non-obvious; never narrate the *what* (well-named identifiers handle that).
   - When choosing between multiple valid approaches, briefly note why this one.
   - Point out important Rust concepts in use (ownership, lifetimes, traits, iterators), especially when the feature might be unfamiliar.
