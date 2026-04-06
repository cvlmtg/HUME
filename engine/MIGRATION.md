# Plan: Wire New Engine to Editor

## Context

HUME has two rendering systems: the old one in `editor/src/ui/` (monolithic `render()` function, `ViewState`, `EditorColors`, `HighlightMap`, `DocumentFormatter`) and the new engine in `engine/` (4-stage pipeline, provider-based, scope-themed, split-pane-ready). The new engine is fully implemented and tested but not yet wired into the editor's event loop. This migration replaces the old rendering code with the new engine, keeping all statusline logic and porting colors to the new theme system.

---

## Step 0: Trim the engine

Two changes to the engine crate before touching the editor.

### 0a. Remove rope ownership from `SharedBuffer`

`SharedBuffer` currently owns a `Rope`, which would require a per-frame copy to stay in sync with the editor's authoritative `Document`. Instead, have `EngineView::render()` accept a `get_rope` closure so each pane resolves its rope from the caller at render time — zero-copy, multi-pane-ready.

- Remove `rope: Rope` from `SharedBuffer`. It keeps `tree: Option<tree_sitter::Tree>` for future syntax highlighting.
- Replace `SharedBuffer::from_str()` and `SharedBuffer::from_rope()` with `SharedBuffer::new() -> Self { Self { tree: None } }`. Add `SharedBuffer::with_tree(tree)` for future use. (No engine tests use the old constructors — safe to remove.)
- Change `EngineView::render()` to accept `get_rope: &dyn Fn(BufferId) -> Option<&ropey::Rope>`.
- Change `PaneRenderCtx` to hold `rope: &'a ropey::Rope` and `tree: Option<&'a tree_sitter::Tree>` instead of `buffer: &'a SharedBuffer`.
- Inside `render()`, for each pane: call `get_rope(pane.buffer_id)` for the rope and look up `self.buffers.get(pane.buffer_id)` for the tree.
- `SharedBuffer`, `BufferId`, `buffers: SlotMap<BufferId, SharedBuffer>`, and `Pane::buffer_id` are all retained — they are the multi-pane bookkeeping structure.

At call time in the editor (single document today):
```rust
engine_view.render(frame.area(), frame.buffer_mut(), &|_id| Some(doc.buf().rope()))
```
For future multi-document:
```rust
engine_view.render(frame.area(), frame.buffer_mut(), &|id| docs.get(id).map(|d| d.buf().rope()))
```

### 0b. Expose `format_buffer_line` as `pub`

The scroll logic (Step 7) needs to count visual rows for a single buffer line from the editor crate. `format_buffer_line` is currently `pub(crate)` — invisible across the crate boundary.

Add a thin public wrapper:
```rust
pub fn count_visual_rows(
    rope: &Rope,
    line_idx: usize,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    scratch: &mut FormatScratch,
) -> usize {
    scratch.display_rows.clear();
    scratch.graphemes.clear();
    format_buffer_line(rope, line_idx, tab_width, whitespace, wrap_mode, &[], scratch);
    scratch.display_rows.len()
}
```

**Files**: `engine/src/pipeline.rs`, `engine/src/format.rs`

---

## Step 1: Unify `Mode` — use `engine::types::EditorMode` directly

The editor's `Mode { Normal, Insert, Command, Search, Select }` + `extend: bool` and the engine's `EditorMode { Normal, Insert, Select, Extend, Command, Search }` represent the same concept. Merge them:

- Delete `editor::Mode` entirely.
- Remove `extend: bool` from `Editor`. Sticky extend is now represented by `EditorMode::Extend`.
- Use `engine::types::EditorMode` everywhere in the editor crate.

**Extend state — two independent mechanisms to preserve:**

1. **Sticky extend** (`self.extend` → `self.mode == EditorMode::Extend`): the persistent toggle via `toggle-extend`. Setting mode to `Extend` replaces the bool.

2. **One-shot ctrl-extend** (`ctrl_extend`): a *per-dispatch local variable*, computed when kitty keyboard protocol detects a Ctrl+motion with no explicit trie binding. This is orthogonal to the mode and must NOT become a mode change. The dispatch site becomes:
   ```rust
   let extend = (self.mode == EditorMode::Extend) || ctrl_extend;
   execute_keymap_command(name, count, extend);
   ```
   `execute_keymap_command(name, count, extend: bool)` keeps its current signature unchanged.

**Specific changes:**
- Key dispatch in `mappings.rs`: `Extend` routes to the same `handle_normal` handler as `Normal`. Add `fn is_normal_mode(mode: EditorMode) -> bool { matches!(mode, Normal | Extend) }`.
- `toggle-extend` command: `ed.mode = EditorMode::Extend` (or toggle back to `Normal`).
- `collapse-and-exit-extend` command: `ed.mode = EditorMode::Normal`.
- `begin_insert_session`: drop `self.extend = false` — setting `self.mode = Insert` implicitly exits `Extend` since a mode holds exactly one variant. Add a comment: `// Mode is SSOT for extend state; transitioning to Insert implicitly clears Extend.`
- `cursor_style`: move from `renderer.rs` to a free function in `editor/src/editor/mod.rs`. Rewrite using `EditorMode::cursor_is_bar()`:
  ```rust
  fn cursor_style(mode: EditorMode) -> SetCursorStyle {
      if mode.cursor_is_bar() { SteadyBar } else { SteadyBlock }
  }
  ```
  `Extend` returns `false` from `cursor_is_bar()`, so it gets `SteadyBlock` — same as `Normal`. Correct.

**Files**: `editor/src/editor/mod.rs`, `editor/src/editor/commands.rs`, `editor/src/editor/mappings.rs`, `editor/src/editor/tests.rs`, `editor/src/ui/statusline.rs`

---

## Step 2: Replace `editor/src/ui/theme.rs` — default theme

Replace the old `EditorColors` flat struct with a function that builds an engine `Theme`. Keep the file name, replace the contents.

`pub(crate) fn build_default_theme() -> engine::theme::Theme` maps old `EditorColors` defaults → scope strings:
- `"ui.cursor"` → white-on-black
- `"ui.cursor.insert"` → same (bar cursor contexts)
- `"ui.selection"` → rgb(68,68,120) bg
- `"ui.cursorline"` → rgb(35,35,45) bg
- `"ui.virtual"` → DarkGray fg (tilde rows)
- `"ui.linenr"` → DarkGray fg
- `"ui.linenr.selected"` → rgb(180,180,180) fg + cursor_line bg
- `"ui.cursor.match"` → bracket match (gold on dark, bold)
- `"ui.selection.search"` → search match (orange on dark) — Helix convention, consistent with `ui.selection` family
- `"ui.whitespace"` → rgb(70,70,80) fg
- `"ui.statusline"` → reversed (base statusline style)
- `"ui.statusline.mode.normal"` / `.insert` / `.extend` / `.search` / `.command` / `.select` → per-mode label colors

These scope names must also appear in `build_default_theme()`'s `styles` map so the `Theme` resolves them, and must be interned via `engine_view.registry` at provider construction time.

**Files**: replace `editor/src/ui/theme.rs` contents

---

## Step 3: Create highlight providers

Replace `editor/src/ui/highlight.rs` (char-offset `HighlightMap`) with engine-compatible `HighlightSource` providers.

**`BracketMatchHighlighter`**: wraps `Arc<RwLock<Vec<(usize, usize, usize)>>>` (line_idx, byte_start, byte_end). The editor writes match data each frame; the provider reads in `highlights_for_line()`. Tier: `BracketMatch`. Scope: `"ui.cursor.match"`, interned at construction.

**`SearchMatchHighlighter`**: same pattern. Tier: `SearchMatch`. Scope: `"ui.selection.search"`. Takes char-offset pairs from `SearchState::matches()`, converts to line-relative byte offsets.

Both use `Arc<RwLock<...>>` for interior mutability. `RwLock` is uncontended (~25ns per op) and idiomatic — do not replace with `UnsafeCell`.

**Files**: new `editor/src/ui/highlight_providers.rs`

---

## Step 4: Statusline → `StatuslineProvider` impl

The statusline currently reads `&Editor` directly. Refactor to a snapshot-based approach:

1. Define `StatuslineSnapshot` struct capturing all statusline data: `mode: EditorMode`, `file_path`, cursor pos `(line, col)`, `kitty_enabled`, `is_dirty`, `search_match_count`, `search_wrapped`, `minibuf`, `status_msg`, `statusline_config`. No `extend` field — mode is `EditorMode` with `Extend` as a variant.
2. Wrap in `Arc<Mutex<StatuslineSnapshot>>`, shared between editor (writes) and provider (reads).
3. `HumeStatusline` struct holds this arc + implements `StatuslineProvider`.
4. Keep all rendering logic (span building, section layout, minibuf cursor) from `statusline.rs` — redirect field accesses from `&Editor` to `&StatuslineSnapshot`. Mode label colors come from the theme via `theme.resolve_by_name(Scope("ui.statusline.mode.normal"))` etc.
5. Per-frame snapshot rebuild (`update_statusline_snapshot()`) is acceptable — cost is a few short string clones per frame, negligible.

**Files**: modify `editor/src/ui/statusline.rs` in place

---

## Step 5: Modify `Editor` struct

**Remove fields**:
- `view: ViewState`
- `colors: EditorColors`
- `highlights: HighlightMap`
- `extend: bool` (mode is now `EditorMode` with `Extend` variant)

**Change field type**:
- `mode: Mode` → `mode: engine::types::EditorMode`

**Add fields**:
- `engine_view: engine::pipeline::EngineView`
- `pane_id: engine::pipeline::PaneId`
- `buffer_id: engine::pipeline::BufferId`
- `bracket_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>` (line_idx, byte_start, byte_end — shared with bracket provider)
- `search_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>` (same shape — shared with search provider)
- `statusline_data: Arc<Mutex<StatuslineSnapshot>>` (shared with statusline provider)

**Add convenience accessors**:
- `fn viewport(&self) -> &ViewportState`
- `fn viewport_mut(&mut self) -> &mut ViewportState`
- `fn pane(&self) -> &Pane` / `fn pane_mut(&mut self) -> &mut Pane`

**Push-based pane sync** (replaces per-frame poll): instead of a `sync_pane_state()` call every frame, update the pane at the point of change:
- `set_mode(mode)` also sets `self.engine_view.panes[self.pane_id].mode = mode`.
- After every `doc.set_selections(...)` call (in `apply_motion`, commands, etc.), convert and push to `self.engine_view.panes[self.pane_id].selections`. Inline the conversion (char offsets → `DocPos` via `char_to_line` + rope byte math) at each site, or extract a helper `fn push_selections_to_pane(&mut self)` called after each mutation.

This eliminates a per-frame O(N log N) selection conversion on idle frames.

**Files**: `editor/src/editor/mod.rs`

---

## Step 6: Rewrite `Editor::open()`

After creating `Document`:
1. `ui::theme::build_default_theme()` → `Theme`
2. `EngineView::new(theme)` + intern `"ui.cursor.match"` and `"ui.selection.search"` scopes via `engine_view.registry`
3. Insert `SharedBuffer::new()` → `buffer_id`
4. Create `Pane` with `buffer_id`, `ViewportState::new(80, 24)`, `WrapMode::Indent { width: 76 }`, `tab_width: 4`, engine `WhitespaceConfig::default()`
5. Register providers: `LineNumberColumn::with_style(0, Hybrid)`, bracket HL, search HL
6. Insert pane → `pane_id`, set `LayoutTree::Leaf(pane_id)`
7. Install statusline provider on `engine_view.statusline`
8. `engine_view.theme.bake(&engine_view.registry)`

**Files**: `editor/src/editor/mod.rs`

---

## Step 7: Port scroll logic to `editor/src/editor/scroll.rs`

Extract to a new submodule `editor/src/editor/scroll.rs` — not inline in `mod.rs`, which is already 662 lines. The scroll functions take clean inputs (`ViewportState`, `&Rope`, `&WrapMode`, etc.) with no dependency on `&Editor`, making them easy to test independently.

Field mapping from old `ViewState` to engine `ViewportState`:
- `scroll_offset` → `top_line`
- `scroll_sub_offset` → `top_row_offset` (u16)
- `col_offset` → `horizontal_offset` (u16)
- `height`/`width` → same names on `ViewportState`

The wrapped scroll path needs to count visual rows per line and find the cursor's sub-row within a line. Use `engine::format::count_visual_rows()` (the public wrapper added in Step 0b) and a `cursor_sub_row()` helper that formats the cursor's line and finds which display row contains the cursor's byte offset.

`scroll.rs` exports:
- `pub(super) fn ensure_cursor_visible(viewport: &mut ViewportState, rope: &Rope, cursor_char: usize, wrap_mode: &WrapMode, tab_width: u8)`
- `pub(super) fn ensure_cursor_visible_horizontal(viewport: &mut ViewportState, rope: &Rope, cursor_char: usize, tab_width: u8)`

Called from `Editor::run()` as:
```rust
scroll::ensure_cursor_visible(&mut self.viewport_mut(), self.doc.buf().rope(), cursor_char, &self.pane().wrap_mode, self.pane().tab_width);
```

**Files**: new `editor/src/editor/scroll.rs`, `editor/src/editor/mod.rs`

---

## Step 8: Rewrite event loop

**`Editor::run()`** becomes:
```
loop {
    sync_dimensions()            // terminal size → viewport width/height
    ensure_cursor_visible()      // scroll::ensure_cursor_visible(...)
    update_highlight_providers() // bracket + search → Arc<RwLock<...>> data
    update_statusline_snapshot() // editor fields → StatuslineSnapshot

    term.draw(|frame| {
        engine_view.render(
            frame.area(),
            frame.buffer_mut(),
            &|_id| Some(doc.buf().rope()),
        );
        if let Some(pos) = compute_cursor_pos() {
            frame.set_cursor_position(pos);
        }
    })

    execute!(stdout(), cursor_style(self.mode))
    event read + dispatch
    // mode and selections are pushed to the pane at point-of-change,
    // not polled here
}
```

No `sync_pane_state()` — mode and selections are kept in sync via push (Step 5).

**`update_highlight_providers()`**: reuses existing bracket-finding and search match logic, converts char-offset results to `(line_idx, byte_start, byte_end)`, writes into the `Arc<RwLock<...>>` shared data.

**`compute_cursor_pos()`**: in `EditorMode::cursor_is_bar()` modes, compute screen (x, y) from viewport + cursor char position. Use `scroll::cursor_sub_row()` (extracted from old `formatter.rs`) to find the display row within the viewport, then walk graphemes for the column.

**Files**: `editor/src/editor/mod.rs`

---

## Step 9: Delete old UI modules

Remove from `editor/src/ui/`:
- `renderer.rs` — replaced by engine pipeline + `cursor_style` moved to `editor/mod.rs`
- `formatter.rs` — replaced by engine format stage + scroll helpers in `scroll.rs`
- `view.rs` — replaced by engine `ViewportState`
- `gutter.rs` — replaced by engine `LineNumberColumn`
- `highlight.rs` — replaced by highlight providers
- `whitespace.rs` — replaced by engine `WhitespaceConfig`

`theme.rs` is kept but its contents are replaced (Step 2).

Update `editor/src/ui/mod.rs` to declare only:
```rust
pub(crate) mod highlight_providers;
pub(crate) mod statusline;
pub(crate) mod theme;
```

**Files**: delete 6 files, update `mod.rs`, replace `theme.rs`

---

## Step 10: Fix all references throughout editor crate

Many files reference the deleted types. Key changes:
- `editor.view.X` → `editor.viewport().X` or `editor.pane().X`
- `editor.view.soft_wrap` → `editor.pane().wrap_mode.is_wrapping()`
- `editor.view.tab_width` → `editor.pane().tab_width as usize`
- `editor.view.scroll_offset` → `editor.viewport().top_line`
- `editor.view.col_offset` → `editor.viewport().horizontal_offset as usize`
- `editor.view.content_width()` → `editor.viewport().width as usize - gutter_width`
- `editor.view.height` → `editor.viewport().height as usize`
- `editor.extend` → `editor.mode == EditorMode::Extend`
- `editor.colors` → removed
- `editor.highlights` → removed

Update `Editor::for_testing()` and builder methods in test code.

**Files**: `editor/src/editor/mod.rs`, `editor/src/editor/commands.rs`, `editor/src/editor/mappings.rs`, `editor/src/editor/tests.rs`, and any other files that import old UI types

---

## Step 11: Migrate / fix tests

- Old renderer snapshot tests → deleted. Engine has its own render tests.
- Old formatter tests → deleted. Engine `format.rs` has its own tests.
- Old view scroll tests → rewrite in `scroll.rs` against the new scroll functions.
- Old highlight tests → replaced by `highlight_providers` tests.
- Statusline tests → adapt from `&Editor` to `&StatuslineSnapshot`.
- `Editor::for_testing()` → update to construct `engine_view` with a `SharedBuffer::new()` and a pane pointing to it; set initial pane mode and selections.

---

## Verification

1. `cargo build` — must compile cleanly
2. `cargo test` — all remaining tests pass
3. Manual: open a file, verify line numbers, cursor movement, scrolling, soft-wrap, bracket match highlight, search highlights, statusline (mode indicator, file name, position, dirty indicator, search count, minibuf)
4. Manual: verify Insert mode bar cursor position is correct
5. Manual: verify kitty protocol indicator works
