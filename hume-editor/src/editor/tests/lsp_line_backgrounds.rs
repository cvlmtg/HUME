// Provider-driven full-row background tint (`set-line-backgrounds!`): the
// `update_line_bg_providers` write side that feeds the new
// `PaneLineBackgrounds` (LINE_BG-kind `DecorationSource`) from the
// `decorations.line_backgrounds` store, and the engine's `row_bg`
// generalization that renders it.
//
// Every test here goes through `Editor::open(None, std::sync::Arc::new(|| {}))` (not `editor_from`'s
// bare `Pane::new`) — `PaneLineBackgrounds` is only registered by
// `build_pane`, same reasoning as `lsp_render.rs`.

use super::*;
use hume_engine::pipeline::RenderContext;
use hume_grid::{Rect, Rgb};

/// Reuses `ui.selection.search` purely as a scope guaranteed to carry a
/// distinct, known `bg` in the embedded snapshot theme — the tint mechanism
/// doesn't care what scope a plugin names.
const TINT_SCOPE: &str = "ui.selection.search";

#[test]
fn line_background_tints_gutter_content_and_trailing_cells() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    // Esc lands the cursor on line 2 ("ghi") — line 0 is tinted but not the
    // cursor's line, so this test isolates the tint from cursorline.
    type_text(&mut ed, "abc\ndef\nghi");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "go" "" (lambda ()
                 (set-line-backgrounds! "git-diff" (current-buffer)
                   (list (list 0 "{TINT_SCOPE}")))))"#
        ),
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let buf = ed.render_to_buf(Rect::new(0, 0, 40, 8));

    let scope = ed
        .view
        .registry
        .get(TINT_SCOPE)
        .expect("set-line-backgrounds! must have interned the scope");
    let expected_bg = ed.view.theme.resolve(scope).bg;
    assert!(expected_bg.is_some(), "sanity: the test scope has a bg");

    // Both paint sites must agree: the gutter cell (row-fill site) and a
    // content grapheme cell (per-grapheme layering site) carry the same
    // tint, and it extends past the 3-char line to a trailing blank cell.
    assert_eq!(buf[(0, 0)].style().bg, expected_bg, "gutter cell");
    assert_eq!(
        buf[(3, 0)].style().bg,
        expected_bg,
        "a content grapheme cell"
    );
    assert_eq!(
        buf[(10, 0)].style().bg,
        expected_bg,
        "a trailing blank cell past the text"
    );

    // The untinted line must not carry it.
    assert_ne!(
        buf[(3, 1)].style().bg,
        expected_bg,
        "an untinted line's content"
    );
}

#[test]
fn line_background_tint_survives_every_wrap_row_of_a_wrapped_line() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    // Line 0 (20 cols, wraps at a narrow width) is tinted; line 1 ("b") is
    // short and holds the cursor after Esc, so cursorline never lands on
    // the tinted line — this test isolates wrap-row persistence from the
    // cursorline-precedence case covered separately below.
    type_text(&mut ed, "aaaaaaaaaaaaaaaaaaaa\nb");
    ed.view.panes[ed.state.focused_pane_id].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 0 }),
        saved: None,
    });
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "go" "" (lambda ()
                 (set-line-backgrounds! "git-diff" (current-buffer)
                   (list (list 0 "{TINT_SCOPE}")))))"#
        ),
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(8, 8); // narrow content width forces the wrap
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let buf = ed.render_to_buf(Rect::new(0, 0, 8, 8));

    let scope = ed.view.registry.get(TINT_SCOPE).expect("interned");
    let expected_bg = ed.view.theme.resolve(scope).bg;

    // Column 0 is always inside the gutter, whatever its width — checking
    // there avoids depending on exact wrap-column arithmetic.
    assert_eq!(
        buf[(0, 0)].style().bg,
        expected_bg,
        "line 0's first wrap row"
    );
    assert_eq!(
        buf[(0, 1)].style().bg,
        expected_bg,
        "line 0's second wrap row"
    );
}

#[test]
fn cursorline_wins_over_the_line_background_tint() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "abc\ndef"); // Esc lands the cursor on line 1 ("def").
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "go" "" (lambda ()
                 (set-line-backgrounds! "git-diff" (current-buffer)
                   (list (list 1 "{TINT_SCOPE}")))))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "sanity: cursor on the tinted line"
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let buf = ed.render_to_buf(Rect::new(0, 0, 40, 8));

    let cursorline_bg = ed.view.theme.ui.cursorline.bg;
    assert!(
        cursorline_bg.is_some(),
        "sanity: this theme's cursorline has a bg"
    );
    let scope = ed.view.registry.get(TINT_SCOPE).expect("interned");
    let tint_bg = ed.view.theme.resolve(scope).bg;
    assert_ne!(
        cursorline_bg, tint_bg,
        "sanity: cursorline and the tint are visually distinct colors"
    );

    assert_eq!(
        buf[(0, 1)].style().bg,
        cursorline_bg,
        "the cursor's own tinted line must show cursorline, not the tint"
    );
}

#[test]
fn line_background_shows_through_when_cursorline_has_no_bg() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    // A from-scratch theme with no `ui.cursorline` entry at all, rather than
    // overriding the snapshot theme's baked `ui.cursorline.bg` post hoc —
    // `Theme::bake` (run by `prepare_frame` whenever a new scope is
    // interned) recomputes every `ui.*` field from the raw map, which would
    // silently undo a direct field override before this test ever renders.
    let styles: std::collections::HashMap<&'static str, hume_engine::types::ResolvedStyle> =
        std::collections::HashMap::from([(
            TINT_SCOPE,
            hume_engine::types::ResolvedStyle {
                bg: Some(Rgb(80, 40, 0)),
                ..Default::default()
            },
        )]);
    ed.view.theme =
        hume_engine::theme::Theme::new(styles, hume_engine::types::ResolvedStyle::default());
    type_text(&mut ed, "abc\ndef"); // Esc lands the cursor on line 1 ("def").
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "go" "" (lambda ()
                 (set-line-backgrounds! "git-diff" (current-buffer)
                   (list (list 1 "{TINT_SCOPE}")))))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "sanity: cursor on the tinted line"
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let buf = ed.render_to_buf(Rect::new(0, 0, 40, 8));

    let scope = ed.view.registry.get(TINT_SCOPE).expect("interned");
    let tint_bg = ed.view.theme.resolve(scope).bg;
    assert_eq!(
        buf[(0, 1)].style().bg,
        tint_bg,
        "with no cursorline bg, the cursor's tinted line must fall through to the tint"
    );
}

/// Regression guard for the decoration-bridge snapshot unification:
/// `update_line_bg_providers` runs in `prepare_frame` step 5, *after* the
/// scroll step, and must read that step's viewport — not the snapshot step 3
/// takes before scrolling (which the sign/inlay-hint/virtual-line/EOL-text
/// bridges deliberately do read; see `decoration_providers.rs`'s
/// `decorated_panes` doc). A ten-line buffer with the cursor on the last
/// line, `scrolloff` 0, and a viewport four content rows tall forces a real
/// scroll during this frame — cursor line 9 minus the scroll target's 3 rows
/// of look-ahead (`scroll.rs::ensure_cursor_visible`) lands `top_line` at 6.
/// Line 8 sits inside that post-scroll viewport (lines 6..11) but well
/// outside the pre-scroll one (0..5) — reachable only if this bridge reads
/// the post-scroll snapshot.
#[test]
fn line_background_reflects_the_post_scroll_viewport_not_the_pre_scroll_one() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.state.settings.scrolloff = 0; // isolate from margin-triggered auto-scroll
    type_text(
        &mut ed,
        "line0\nline1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9",
    );
    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        9,
        "sanity: cursor lands on the last line, off-screen at the default top_line=0"
    );

    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "go" "" (lambda ()
                 (set-line-backgrounds! "git-diff" (current-buffer)
                   (list (list 8 "{TINT_SCOPE}")))))"#
        ),
    );
    type_cmd(&mut ed, ":go");

    let pid = ed.state.focused_pane_id;
    frame(&mut ed, 20, 5); // rect height 5 → 4 content rows, forces the scroll

    assert_eq!(
        ed.view.panes[pid].viewport.top_line, 6,
        "sanity: the cursor forced this frame's own scroll, landing top_line \
         where line 8 is visible but line 0's default viewport never was"
    );

    let by_line = ed.state.panes.render[pid]
        .line_backgrounds
        .read()
        .unwrap()
        .clone();
    assert!(
        by_line.contains_key(&8),
        "line 8's tint must survive into the post-scroll viewport this bridge reads"
    );
}
