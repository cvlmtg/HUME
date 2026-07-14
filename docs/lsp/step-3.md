# LSP Step 3 — UI surfaces (task cards)

Generic, Steel-scriptable widgets plus store-fed render wiring. The engine primitives mostly exist (reserved `HighlightTier::Diagnostic`, `SignColumn`/`SignSource`, `VirtualLineSource`, `InlineDecoration`, `OverlayProvider`) — LSP is their first client, not their owner. Read `docs/LSP.md` (hub) first.

Shared context for all U-cards:
- The feeding pattern is always the `SharedHighlighter` split (`hume-editor/src/ui/highlight_providers.rs` module docs): a per-pane `Arc<RwLock<…>>` the editor **writes once per frame** (from the C9/B5 stores, visible range only) and the engine provider **reads per line**. Steel never appears on either side of that split.
- Providers register per-pane in `build_pane` (`hume-editor/src/ui/mod.rs`).
- Rendering tests use `insta` inline snapshots (existing engine test convention) driven through the editor test harness.
- Scope names used by new decorations must exist in the default theme(s) — check `runtime/themes/` and add scopes with sensible styles in the same task that introduces them. Remember the theme rule: themes must define `ui.cursor.insert` (LESSONS) — i.e. missing scopes fail quietly; test styled output, not just geometry.

---

### U1 — Diagnostic underlines + extra-highlights wiring

**Goal** — diagnostics render as styled underlines; B5's generic `set-extra-highlights!` store renders through the same mechanism.

**Depends** — C9, B5. **Unlocks** — the visible half of the Step 3 milestone.

**Files** — `hume-editor/src/ui/highlight_providers.rs` (new provider or a third `SharedHighlighter` instance + a scope-carrying variant), `ui/mod.rs` (`build_pane` registration), the per-frame write site (`update_highlight_providers` — find its caller in `lifecycle.rs`), `hume-engine/src/style/highlight.rs` + `providers.rs` (only if the OQ default's new `Extra` tier is confirmed).

**Read first** — `SharedHighlighter` + `PaneHighlights` + `update_highlight_providers` (the whole feeding pipeline); `HighlightTier` ordering semantics (`hume-engine/src/providers.rs` + `style/highlight.rs` `TIER_COUNT` arrays); hub OQ *`set-extra-highlights!` tier* default.

**Shape** — two registrations in `build_pane`:
1. Diagnostics: a `SharedHighlighter`-like provider at `HighlightTier::Diagnostic`, but **per-severity scopes** — the existing struct carries one `ScopeId`; diagnostics need `diagnostic.error` / `diagnostic.warning` / `diagnostic.info` / `diagnostic.hint`. Extend the data tuple to carry a `ScopeId` per range (`Vec<(line, start, end, ScopeId)>`) — a new small provider struct beside `SharedHighlighter`, same Arc pattern (don't force the existing two registrations through the richer shape).
2. Extra-highlights: same provider struct at the tier the OQ default names (`Extra` between `Syntax` and `SearchMatch` — gate: confirm here, renumbering discriminants + `TIER_COUNT` arrays + this hub row → Decisions).

Per-frame write (visible lines only): C9 `for_range` (severity floor knob `lsp.diagnostics-severity-floor`, default Hint = everything) → line-relative **byte** offsets (providers speak line-relative bytes — convert char offsets via the rope, per the `HighlightSource` contract) → severity scope; B5 extra-highlights store → same shape with its own scopes.

**Tests** — tier 2 + snapshots: a diagnostic renders with the error scope's style (insta snapshot of the styled row); severity floor hides Hint when raised; multi-line diagnostic underlines every covered line; extra-highlight from a Steel `set-extra-highlights!` call renders at the right tier (search match visually beats it; it beats syntax — snapshot with overlapping spans); zero diagnostics = zero provider output (no empty-vec allocation churn per frame — reuse the write-side buffer).

**Done when** — milestone-style manual check: rust-analyzer error → underline appears, styled per theme; `:set` of the severity floor changes what's shown. Theme scopes added to the shipped themes.

**Traps**
- Providers get **line-relative byte** offsets (the `HighlightSource` contract) — the C9 store holds buffer-wide char offsets; convert at the write site, not in the provider.
- Write the Arcs after scroll resolution, before draw — exactly where `update_highlight_providers` already runs; don't invent a second write point.
- The engine debug_asserts sorted, non-overlapping spans per line — C9's order gives you sorted starts; overlapping same-line diagnostics must be merged or split at the write site.

**Size** — ~180 source + ~150 test lines.

---

### U2 — Diagnostic gutter signs

**Goal** — per-line severity signs in the gutter; first real `SignColumn` registration; B5's `set-signs!` rides the same column.

**Depends** — C9, B5. **Unlocks** — F-visible polish; `ProviderSet::remove`'s first consumer if the toggle is included.

**Files** — `hume-editor/src/ui/` (a `SignSource` impl over the shared-Arc pattern), `build_pane` (register a `SignColumn` + sources), engine `hume-engine/src/builtins/sign_column.rs` (read-only — the machinery exists).

**Read first** — `sign_column.rs` end to end: `Sign { text, scope, priority }`, `SignSource::sign_for_line(line, ctx) -> Option<Sign>` (called per row per frame — must be a cheap map lookup), `SignColumn` merge-by-priority semantics; the ROADMAP row *`ProviderSet::remove` unwired* (this card may wire it).

**Shape** — one `SignSource` over a per-pane `Arc<RwLock<HashMap<usize /*line*/, Sign>>>` written per frame from C9 (visible range; highest severity wins per line, e.g. `●` styled by severity scope); a second `SignSource` over B5's signs store (plugin signs). Both registered on one `SignColumn` — priority rules already merge them (diagnostics priority 10, plugin default 0, knob later if needed).

Optional (decide here, it's cheap): `:set`-toggled visibility via `ProviderSet::remove` + captured `ProviderId`s — if included, that resolves the ROADMAP "unwired" row (update ROADMAP).

**Tests** — snapshot: error line shows the sign styled; error beats warning on the same line; plugin sign via `(set-signs! …)` appears; sign column absent when no signs exist (check gutter width math — `SignColumn` width behavior on empty).

**Done when** — manual: error file shows gutter dots; theme scopes shipped.

**Traps**
- `sign_for_line` runs per visible row per frame — the write side pre-computes the per-line winner; the source is a `HashMap` get, nothing more.
- 1–2 cell sign text (wider truncates) — pick single-glyph defaults.

**Size** — ~120 source + ~100 test lines.

---

### U3 — Statusline diagnostics element

**Goal** — error/warning counts for the focused buffer in the statusline, e.g. `✘ 3 ⚠ 12`, hidden when zero.

**Depends** — C9. **Unlocks** — Step 3 milestone.

**Files** — `hume-editor/src/ui/statusline.rs` (`StatusElement` variant + render arm + `Display` name), `hume-scripting/src/builtins/statusline.rs` (accept the new element name in `configure-statusline!` parsing — check whether names parse from the `Display` impl already; if so this is free).

**Read first** — `StatusElement` enum + its render match + the `Display`/parse round-trip; `builtins/statusline.rs` name validation; hub OQ *$/progress* default (message-log only — **no spinner element here**, that's Future).

**Shape** — `StatusElement::Diagnostics`: reads C9 `counts(focused_bid)` **directly in Rust** (statusline renders per frame; the Steel `(diagnostic-counts)` builtin is for plugins, never for rendering — the hub render guardrail). Empty string when both counts are zero (the `Selections` element shows the collapse-when-empty precedent). Styled via severity scopes if the statusline supports per-element scopes; else plain text (match existing element conventions — read two neighbouring arms before choosing).

**Tests** — snapshot with counts / without; element name round-trips through `configure-statusline!`; counts update across an edit that fixes an error (tier 2 with the double).

**Done when** — default statusline layout includes it (right section) and a fresh editor shows nothing until diagnostics exist.

**Traps** — resist reading through Steel here; the whole point of C9-in-Rust is this render path.

**Size** — ~60 source + ~60 test lines.

---

### U4 — Cursor-anchored popup widget

**Goal** — `(show-popup! text #:anchor 'cursor)` / `(close-popup!)`: a floating text panel at the cursor with flip/clamp at viewport edges. Serves hover (F1), signature help (F7), and U7's menu geometry.

**Depends** — B1 (nothing else — deliberately early-ish and generic). **Unlocks** — U5, U7, F1, F7.

**Files** — `hume-editor/src/ui/popup.rs` (new `OverlayProvider`), `build_pane` (register), builtins in a new `hume-scripting/src/builtins/ui.rs` (register in `builtins/mod.rs`).

**Read first** — `ui/completion_overlay.rs` **for the cell-painting approach only** — it's statusline-anchored; its geometry does not transfer. `OverlayProvider` trait (`render(pane_rect, theme, buf)`, `is_active`); how the completion overlay's `Arc<RwLock<Option<CompletionView>>>` state is shared between editor and provider (same pattern here); cursor screen position: `cursor::screen_pos` (`hume-editor/src/editor/cursor.rs` — verify module path by symbol search).

**Shape**
```rust
pub(crate) struct PopupState {
    pub lines: Vec<String>,        // pre-wrapped by the write side to max width
    pub anchor: (u16, u16),        // screen cell of the primary cursor, pane-relative
}
pub(crate) struct PopupOverlay { data: Arc<RwLock<Option<PopupState>>> }
```
Geometry rules (document in the module; F1/F7/U5/U7 all inherit them): preferred placement below-right of the anchor; **flip** above when the space below is smaller than the popup and above is larger; **clamp** horizontally into the pane; max width = min(60, pane width − 4); max height = ⅓ pane height (the hub hover OQ default's threshold — taller content is the *caller's* problem: F1 overflows to the drawer). Dismissal is the caller's job (`close-popup!`) — the widget holds no keymap; F-cards close on cursor move / mode change via hooks.

**Tests** — snapshots: popup below cursor; flipped above near the bottom edge; clamped at the right edge; multi-byte/emoji content width (unicode-width, not char count); `close-popup!` clears; popup never draws outside `pane_rect`.

**Done when** — `(show-popup! "hello\nworld")` renders at the cursor and closes, manually and in snapshots.

**Traps**
- Wrap text on the **write side** (Steel-call time), not in `render` — render is per-frame.
- `OverlayProvider::render` gets the raw ratatui buffer; overlays paint last (z-order = registration order) — register after the completion overlay so LSP popups win.
- Don't add borders/padding options v1 — one look, theme-scoped (`ui.popup` background scope; add to themes).

**Size** — ~200 source + ~150 test lines.

---

### U5 — Selection menu widget

**Goal** — `(show-menu! items on-select)` / `(close-menu!)`: U4's popup with a selectable row list. Serves code actions (F9).

**Depends** — U4. **Unlocks** — F9.

**Files** — `ui/popup.rs` (menu = popup variant), key handling: a narrow intercept while a menu is open (see Shape), builtins in `builtins/ui.rs`.

**Read first** — U4's state plumbing; how Insert-mode completion keys will be routed in U7 (write these two cards' key story together — same problem, one convention); the pending-keys dispatch (`editor/keymap/` — how a mode consumes keys before the trie; search `pending_keys`, see LESSONS on Ctrl-key interior dispatch).

**Shape**
```scheme
(show-menu! '("Extract function" "Inline variable") (lambda (idx) …))  ; idx = #f on dismiss
```
Menu state = popup state + `selected: usize` + the Steel callback. Keys while open (intercepted **before** normal dispatch, Normal mode only — menus don't open from Insert in v1): `j`/`k`/`Down`/`Up` move, `Enter` confirms (queue `(callback idx)`, close), `Esc` dismisses (`(callback #f)`). Everything else falls through to normal dispatch *after closing* (a stray key dismisses — predictable, no modal trap).

**Tests** — tier 3: select second item → callback gets 1; Esc → `#f`; selection wraps or clamps (pick clamp; document); snapshot of the highlighted row (`ui.menu.selected` scope); stray key dismisses and still executes.

**Done when** — a scripted menu drives a selection end-to-end in tests and manually.

**Traps**
- The key intercept must be a **guarded early-return in the key path**, not a new EditorMode (mode churn leaks into `on-mode-change`, statusline, cursor shape — a menu is transient chrome).
- One callback call, exactly once, then drop the closure (B2's one-shot discipline).

**Size** — ~150 source + ~130 test lines.

---

### U6 — Class B bottom drawer (minimal) + location list

**Goal** — the M12 drawer scoped to LSP's needs: full-width bottom panel, auto-sized (≤ ~50 % height), read-only, scrollable, with a built-in location-list mode: `(show-drawer-list! items on-select)` — `(file line col text)` rows, `j`/`k`/`Enter`-to-jump (via B6 `goto-location!`), `Esc` closes. Serves references (F6), multi-result goto (F2), `:diagnostics` (F4), hover overflow (F1).

**⚠ Least-precedented card in Step 3.** Panes, gutters, statusline exist; a horizontal chrome band below the pane grid does not. Budget extra time; if the engine's pane-area partitioning fights you, stop and report rather than force it.

**Depends** — U4 conventions, B6. **Unlocks** — F1/F2/F4/F6.

**Files** — engine: wherever `EngineView::pane_area` partitions the terminal rect (search `pane_area`; the tab-bar handling shows how a chrome band reserves rows — mimic it for a bottom band); editor: `ui/drawer.rs` (state + render), key intercept (same convention as U5), builtins in `builtins/ui.rs`.

**Read first** — how the tab bar reserves vertical space (the hub orientation row *Event loop* names `prepare_frame`'s shared-rect partitioning comment — read that block); `docs/ROADMAP.md` M12 section (what the full drawer will be — build the subset compatibly, don't preclude it); U5's key-intercept convention; `open_read_only_view` (the poor-man's alternative this replaces — understand why it's insufficient: focus steal + buffer-list pollution).

**Shape**
```scheme
(show-drawer-list! items on-select)   ; items: list of (list path line col text)
(close-drawer!)
```
```rust
pub(crate) struct DrawerState {
    pub rows: Vec<DrawerRow>,      // { loc: (PathBuf, u32, u32), display: String }
    pub selected: usize,
    pub scroll: usize,
}
```
Height = min(rows + 1, terminal_height / 2). Rendering: engine band render (like tab bar) or an `OverlayProvider` painted over the bottom rows — **prefer the band** (real space reservation; panes shrink correctly, no cell fights); if the band requires deep engine surgery, the overlay is the acceptable v1 fallback — decide after the *Read first* and record which in the commit. Focus stays on the pane; drawer keys are the U5-style intercept while open (`j/k` move + auto-scroll, `Enter` = `goto-location!` on the row **and keep the drawer open** (Helix-style browse), `Esc` closes).

**Tests** — snapshots: drawer under the pane grid, selected row styled, long list scrolled; `Enter` jumps (buffer + cursor assert) with a jump-list entry (B6), drawer still open; `Esc` restores full-height panes; resize while open re-clamps height.

**Done when** — a 50-row synthetic list browses smoothly manually; pane content above stays correct (scroll math unaffected — the band changes pane height, which the existing viewport math already handles).

**Traps**
- Do not implement M12 (no filtering, no multiple drawer kinds, no focus mode) — rows + select + jump, done.
- Pane height changes when the drawer opens — `ensure_cursor_visible` consequences are the *existing* resize path's job; open/close should reuse the resize handling, not duplicate it.
- Keep `DrawerRow.display` pre-formatted (Steel formats; Rust paints) — no formatting per frame.

**Size** — ~250 source + ~180 test lines (plus engine-band plumbing if that route is taken — report if it exceeds ~150 more).

---

### U7 — In-buffer completion menu + dispatch

**Goal** — the Insert-mode completion flow: manual trigger (`Ctrl+Space`) + B7 trigger chars open a U4-geometry menu at the cursor showing B8's top-N; `Tab`/`Down`/`Up` navigate, `Enter` accepts, `Esc` dismisses, typing narrows.

**Depends** — B8, U4, B7. **Unlocks** — F3.

**Files** — `ui/popup.rs` (completion menu = U5 menu + doc column), Insert-mode key routing (`editor/mappings/` insert path), `core:lsp`'s `plugin.scm` (`Ctrl+Space` binding → a `lsp-completion-trigger` **named command** so plugins can rebind), builtins already exist (B8).

**Read first** — U5's intercept (this one runs in **Insert** mode — stricter: printable chars must still self-insert *and* refilter); `editor/completion/mod.rs` `CompletionState` (reuse the selection bookkeeping if it fits — the card title says "where it fits": judge after reading; a parallel small struct is fine if minibuffer coupling is awkward); LESSONS on Ctrl-key interior dispatch (`pending_keys`).

**Shape** — while a completion session (B8) is active in Insert mode: printable key → self-insert as normal **then** `completion-update-filter!` with the token text (the session's anchor gives the token start); `Tab`/`Down` next, `Shift+Tab`/`Up` prev, `Enter` → `completion-accept!` (falls through to newline when no session), `Esc` → dismiss and stay in Insert; menu rows from `(completion-top 8)` re-materialized after each filter change (write-side, not per frame). Backspacing past the anchor dismisses.

**Tests** — tier 3 scripted: trigger → menu appears with top items; typing narrows (menu content assert); `Enter` applies the selected item's edit and closes; `Esc` leaves the typed text intact; `Enter` with no session inserts a newline (the fall-through matters); backspace-past-anchor dismisses; minibuffer completion (`:e <Tab>`) untouched (regression test).

**Done when** — manual: `Ctrl+Space` in a rust file offers rust-analyzer completions and accepting one lands the edit. (Needs F3's Steel side for the request — until then, test with a scripted `completion-begin!`; the *manual* proof moves to F3's card.)

**Traps**
- `Enter`'s dual role is the classic breakage — the fall-through test is mandatory (see LESSONS: `key_enter()` in tests, not `key('\n')`).
- Don't let the menu intercept swallow `Ctrl+…` chords Insert mode already binds — intercept only the keys listed, fall through everything else.
- Re-request-on-narrowing when `isIncomplete` is F3's job (Steel), not the widget's.

**Size** — ~200 source + ~200 test lines.

---

### U8 — Inline diagnostics (`VirtualLineSource`) + the deferred rewiring

**Goal** — virtual text lines under diagnostic lines (first real `VirtualLineSource`), which **triggers the deferred editor rewiring** from the ROADMAP decision *Virtual-line-aware scroll/cursor math scope*: thread `&ProviderSet` through the cursor/scroll math so virtual rows count.

**Depends** — C9, B5, U1 (scopes exist). **Unlocks** — F4 polish; unblocks any future virtual-line feature (git blame, code lens).

**Two sub-tasks — land as separate commits:**

**U8a — the rewiring (pure plumbing, zero behavior change).** Read the ROADMAP decision row first — it names the contract (`format::display_rows_for_line` / `RowsBreakdown {before, content, after}` is the engine SSOT, already tested). Thread `&ProviderSet` into: `cursor::screen_pos`, `cursor::screen_to_char_offset`, `scroll::ensure_cursor_visible`, `scroll::scroll_cursor_to_row`, `scroll::scroll_backward_from_cursor` (~10 interlinked functions once callers are counted — the row names them as heavily tested; run their test files after every mechanical step). With no `VirtualLineSource` registered, `RowsBreakdown::total() == content` everywhere, so **every existing test must pass unchanged** — that's the whole verification story for U8a. `ViewportState.top_row_offset` keeps indexing content rows only (virtual rows never partially scroll — the decision row says so).

**U8b — the source.** A `VirtualLineSource` over B5's virtual-lines store (per-pane Arc, U1 pattern): Steel formats the visible diagnostics **on change** (an `on-diagnostics-changed` + viewport-change handler in F-land renders strings into `set-virtual-lines!` — bounded and cached; never per frame). Rust renders whatever's in the store. Styled segments via the `VirtualLine.segments` scope ranges (`diagnostic.error` etc.).

**Read first** — the ROADMAP decision row (verbatim constraints); `format::display_rows_for_line` + `RowsBreakdown` tests (engine); `VirtualLineSource` contract in `providers.rs` (called during scroll/cursor accounting, not just render — must be a cheap lookup).

**Tests** — U8a: the existing suites, unchanged, green — plus one new synthetic-provider test per rewired function (cursor under a virtual line lands correctly; `ensure_cursor_visible` accounts for virtual rows stealing viewport space). U8b: snapshot with a virtual line under an error line; scrolling over it (cursor never lands *on* a virtual row); store cleared → rows disappear.

**Done when** — a diagnostic shows its message under the line, cursor/scroll behave at the boundary, and `git log` shows two commits (plumbing, then feature).

**Traps**
- Do not merge the sub-tasks: a behavior bug in a combined commit is un-bisectable across ~10 touched math functions.
- `virtual_lines()` is called in scroll math paths — per-call work must be a map lookup; the formatting already happened in Steel at signal time.
- Cursor motion must skip virtual rows (they own no buffer positions) — `screen_to_char_offset` maps clicks on a virtual row to its anchor line (decide + test; mouse goes through it).

**Size** — U8a ~150 lines churn across ~10 functions + tests; U8b ~120 source + ~120 test lines. The biggest U-card; it's two sessions.

---

### U9 — Inlay-hint rendering

**Goal** — `InlineDecoration` impl over B5's inlay-hint store, registered in `build_pane`. The pipeline already collects `decorations_for_line` per line (`hume-engine/src/pipeline/pane_render.rs`) — store + registration only. F10 renders through this with zero Rust changes.

**Depends** — B5. **Unlocks** — F10.

**Files** — `ui/` (small provider struct, U1's Arc pattern), `build_pane`.

**Read first** — `InlineDecoration` + `InlineInsert { byte_offset, text, scope }` (`hume-engine/src/providers.rs` — note: byte offset **within the line**, caller sorts); the consumption loop in `pipeline/pane_render.rs`; `hume-engine/src/format.rs` for how inline inserts interact with wrapping (verify: do inserts affect wrap width computation? read before assuming — if they do, note it in the card-completion commit; it doesn't block this card since hints are short).

**Shape** — write side per frame (visible lines): B5 store `(char_pos, text, before/after)` → line + line-relative byte offset via the rope; scope `ui.inlay-hint` (add to themes, dimmed). Provider: `HashMap<usize, Vec<InlineInsert>>` behind the Arc; `decorations_for_line` = clone-out of the entry (small strings, fine) or extend-from-slice into `out`.

**Tests** — snapshot: `: i32`-style hint rendered dimmed mid-line at the right cell (emoji-containing line to prove byte-offset math); before vs after placement; hint on a wrapped line (whatever `format.rs` does — snapshot it so behavior is pinned); store cleared → gone.

**Done when** — scripted `set-inlay-hints!` renders manually; snapshots pinned.

**Traps**
- `byte_offset` is line-relative bytes (same contract as highlights) — convert from the store's char positions at the write site.
- Hints between grapheme clusters: char positions from the store land on char boundaries by construction; don't do arithmetic on them.

**Size** — ~100 source + ~100 test lines.
