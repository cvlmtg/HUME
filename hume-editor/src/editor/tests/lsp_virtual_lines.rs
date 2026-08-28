// Inline diagnostics virtual lines: the
// `update_virtual_line_providers` write side that feeds the new
// `PaneVirtualLines` (VIRTUAL_LINE-kind `DecorationSource`) from the
// `decorations.virtual_lines` store, over the virtual-row-aware scroll/cursor
// plumbing.
//
// Every test here goes through `Editor::open(None, std::sync::Arc::new(|| {}))` (not `editor_from`'s
// bare `Pane::new`) — `PaneVirtualLines` is only registered by `build_pane`,
// same reasoning as `lsp_render.rs`.

use super::*;
use hume_engine::pipeline::RenderContext;
use hume_grid::{Rect, Rgb};

/// The synced `VirtualLine`s filed under `line` on the focused pane, after a
/// `prepare_frame` — the same read `clearing_the_store_removes_the_virtual_line_next_frame`
/// does, generalized to inspect segments rather than just presence.
fn virtual_lines_at(ed: &Editor, line: usize) -> Vec<hume_engine::providers::VirtualLine> {
    let pid = ed.state.focused_pane_id;
    ed.state
        .panes
        .render
        .get(pid)
        .unwrap()
        .virtual_lines
        .read()
        .unwrap()
        .get(&line)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn virtual_line_renders_after_its_anchor_line() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "let x = 5\nlet y = 10");
    let bid = ed.focused_buffer_id();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (hash 'line 0 'text "^ unused variable" 'scope "diagnostic.warning")))))"#,
    );
    type_cmd(&mut ed, ":go");
    let _ = bid;

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn scroll_over_a_virtual_line_pushes_the_next_line_down_correctly() {
    // Snapshot proof (not a numeric row assertion — `cursor::content_pos`'s
    // internals aren't reachable from this integration-style test module):
    // with a virtual line inserted after line 0, moving the cursor onto
    // line 1 ('bbb') must still show "bbb" directly below the virtual
    // line's own row — the row-counting fix must correctly push line
    // 1's content down by the one stolen row, never overlap or skip it.
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "aaa\nbbb\nccc");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (hash 'line 0 'text "hint")))))"#,
    );
    type_cmd(&mut ed, ":go");

    // `Esc` leaves the cursor on line 2 ('ccc'); move up twice then down
    // once to land on line 1 through normal motions.
    ed.feed_key(key_up());
    ed.feed_key(key_up());
    ed.feed_key(key_down());
    let bid = ed.focused_buffer_id();
    let cursor_char = ed.current_selections().primary().head();
    assert_eq!(
        ed.state.buffers.get(bid).text().char_to_line(cursor_char),
        1,
        "sanity: cursor on line 1"
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn before_anchored_virtual_line_renders_above_its_line() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "let x = 5\nlet y = 10");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "git-diff" (current-buffer)
               (list (hash 'line 1 'anchor 'before 'text "- deleted line")))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn per_segment_scopes_style_the_virtual_lines_text() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "let x = 5");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (hash 'line 0 'text "let x"
                           'segments (list (list 0 3 "keyword") (list 4 5 "string")))))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn scope_becomes_the_line_s_base_scope_segments_stay_sparse() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "let x = 5");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (hash 'line 0 'text "abcdef" 'scope "base"
                           'segments (list (list 2 4 "kw")))))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let base = ed.view.registry.get("base").expect("base scope interned");
    let kw = ed.view.registry.get("kw").expect("kw scope interned");
    let lines = virtual_lines_at(&ed, 0);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].segments,
        vec![(2, 4, kw)],
        "'segments passes through sparse — the engine, not the editor, fills gaps from base_scope"
    );
    assert_eq!(
        lines[0].base_scope,
        Some(base),
        "'scope becomes base_scope, the row's fallback and background"
    );
}

#[test]
fn no_segments_yields_an_empty_segment_list() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "let x = 5");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (hash 'line 0 'text "hint" 'scope "base")))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let base = ed.view.registry.get("base").expect("base scope interned");
    let lines = virtual_lines_at(&ed, 0);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].segments,
        Vec::new(),
        "no 'segments — the whole row falls back to base_scope, nothing to name as a segment"
    );
    assert_eq!(lines[0].base_scope, Some(base));
}

#[test]
fn no_scope_falls_back_to_ui_virtual_as_the_base_scope() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "let x = 5");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (hash 'line 0 'text "hint")))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let ui_virtual = ed
        .view
        .registry
        .get("ui.virtual")
        .expect("ui.virtual interned as the fallback base_scope");
    let lines = virtual_lines_at(&ed, 0);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].base_scope,
        Some(ui_virtual),
        "no 'scope must still populate base_scope (with ui.virtual) — never leave it None, \
         or a theme that gives ui.virtual a bg would tint this row's text but not its \
         gutter/trailing cells"
    );
}

/// A theme where `ui.virtual` itself carries a `bg` — none of the bundled
/// themes do, but a custom one might, and a scope-less virtual line's row
/// fill must track it exactly like an explicit `'scope` would.
fn theme_with_tinted_ui_virtual() -> hume_engine::theme::Theme {
    hume_engine::theme::loader::parse_theme(
        r##"
        "ui.virtual" = { fg = "#808080", bg = "#ff00ff" }
        "ui.background" = { fg = "#ffffff", bg = "#000000" }
        "##,
    )
    .expect("inline test theme must parse")
}

#[test]
fn generic_virtual_line_with_no_scope_tints_the_full_row_when_ui_virtual_has_a_bg() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = theme_with_tinted_ui_virtual();
    type_text(&mut ed, "hello");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (hash 'line 0 'text "V" 'anchor 'before)))))"#,
    );
    type_cmd(&mut ed, ":go");

    let buf = ed.render_to_buf(Rect::new(0, 0, 40, 8));

    let virtual_bg = Some(Rgb(0xff, 0x00, 0xff));
    // Row 0 is the virtual line (anchored `'before` line 0). Column 20 sits
    // well past the 1-char text but short of the right edge.
    assert_eq!(buf[(0, 0)].style().bg, virtual_bg, "gutter cell");
    assert_eq!(
        buf[(20, 0)].style().bg,
        virtual_bg,
        "a cell past the virtual line's own text — the row fill, not just the \
         per-grapheme style, must carry ui.virtual's bg"
    );
    assert_eq!(buf[(39, 0)].style().bg, virtual_bg, "window border");
}

/// Reuses `ui.selection.search` purely as a scope guaranteed to carry a
/// distinct, known `bg` in the embedded snapshot theme — same convention as
/// `lsp_line_backgrounds.rs`'s `TINT_SCOPE`.
const TINT_SCOPE: &str = "ui.selection.search";

#[test]
fn virtual_line_background_tints_gutter_content_and_trailing_cells() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "hello");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (set-virtual-lines! "git-diff" (current-buffer)
                   (list (hash 'line 0 'text "V" 'anchor 'before 'scope "{TINT_SCOPE}")))))"#
        ),
    );
    type_cmd(&mut ed, ":go");

    let buf = ed.render_to_buf(Rect::new(0, 0, 40, 8));

    let scope = ed
        .view
        .registry
        .get(TINT_SCOPE)
        .expect("set-virtual-lines! must have interned the scope");
    let expected_bg = ed.view.theme.resolve(scope).bg;
    assert!(expected_bg.is_some(), "sanity: the test scope has a bg");

    // Row 0 is the virtual line (anchored `'before` line 0); row 1 is the
    // real "hello" content line. Column 20 sits well past the 1-char virtual
    // text but short of the right edge — exactly where the bug this test
    // guards against left the row untinted.
    assert_eq!(buf[(0, 0)].style().bg, expected_bg, "gutter cell");
    assert_eq!(
        buf[(20, 0)].style().bg,
        expected_bg,
        "a cell past the virtual line's own text"
    );
    assert_eq!(
        buf[(39, 0)].style().bg,
        expected_bg,
        "the rightmost cell, at the window border"
    );
    assert_ne!(
        buf[(20, 1)].style().bg,
        expected_bg,
        "the real content line below must not carry it"
    );
}

#[test]
fn virtual_line_with_empty_text_still_renders_its_background_bar() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "hello");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (set-virtual-lines! "git-diff" (current-buffer)
                   (list (hash 'line 0 'text "" 'anchor 'before 'scope "{TINT_SCOPE}")))))"#
        ),
    );
    type_cmd(&mut ed, ":go");

    let buf = ed.render_to_buf(Rect::new(0, 0, 40, 8));

    let scope = ed
        .view
        .registry
        .get(TINT_SCOPE)
        .expect("set-virtual-lines! must have interned the scope");
    let expected_bg = ed.view.theme.resolve(scope).bg;

    // No text means no graphemes at all (`segment_virtual_row` emits one
    // grapheme per cluster) — the row-fill is the only thing that can paint
    // this row, so its presence here proves the fill runs independently of
    // content.
    assert_eq!(buf[(0, 0)].style().bg, expected_bg, "gutter cell");
    assert_eq!(buf[(20, 0)].style().bg, expected_bg, "content cell");
    assert_eq!(buf[(39, 0)].style().bg, expected_bg, "window border");
}

#[test]
fn segments_touching_both_ends_yield_no_zero_length_filler() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "let x = 5");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (hash 'line 0 'text "abcdef"
                           'segments (list (list 0 3 "a") (list 3 6 "b")))))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let a = ed.view.registry.get("a").expect("a scope interned");
    let b = ed.view.registry.get("b").expect("b scope interned");
    let lines = virtual_lines_at(&ed, 0);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].segments,
        vec![(0, 3, a), (3, 6, b)],
        "segments already covering the whole text must produce no zero-length filler"
    );
}

#[test]
fn clearing_the_store_removes_the_virtual_line_next_frame() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    let scope = ed.view.registry.intern("ui.virtual");
    ed.state.config.decorations.set_virtual_lines(
        "linter".to_string(),
        bid,
        vec![crate::editor::decorations::VirtualLineEntry {
            pos: 0,
            text: "hint".to_string(),
            before: false,
            scope,
            segments: Vec::new(),
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focused_pane_id;
    let has_line_before = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .virtual_lines
        .read()
        .unwrap()
        .values()
        .any(|v| !v.is_empty());
    assert!(
        has_line_before,
        "sanity: virtual line present before clearing"
    );

    ed.state
        .config
        .decorations
        .set_virtual_lines("linter".to_string(), bid, vec![]);
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let has_line_after = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .virtual_lines
        .read()
        .unwrap()
        .values()
        .any(|v| !v.is_empty());
    assert!(
        !has_line_after,
        "virtual line must be gone once the store is cleared"
    );
}

/// Two sources anchored to the same line must render in alphabetical
/// source-name order, not the order they happened to call
/// `set-virtual-lines!` in — `SourceStore::set` keeps a buffer's sources
/// sorted ascending by name for exactly this: virtual lines have no
/// per-line collapse (unlike signs/EOL text/line backgrounds), so
/// whichever order the store iterates in *is* the render order. Setting
/// "zzz" before "aaa" here would put "zzz" first if `set` fell back to
/// find-or-push instead of a sorted insert.
#[test]
fn same_line_virtual_lines_from_two_sources_order_alphabetically_by_source() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    let scope = ed.view.registry.intern("ui.virtual");
    let entry = |text: &str| crate::editor::decorations::VirtualLineEntry {
        pos: 0,
        text: text.to_string(),
        before: false,
        scope,
        segments: Vec::new(),
    };
    ed.state
        .config
        .decorations
        .set_virtual_lines("zzz".to_string(), bid, vec![entry("from zzz")]);
    ed.state
        .config
        .decorations
        .set_virtual_lines("aaa".to_string(), bid, vec![entry("from aaa")]);

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let texts: Vec<String> = virtual_lines_at(&ed, 0)
        .iter()
        .map(|v| v.text.clone())
        .collect();
    assert_eq!(
        texts,
        vec!["from aaa".to_string(), "from zzz".to_string()],
        "virtual lines must render in ascending source-name order regardless \
         of which source called set-virtual-lines! first"
    );
}
