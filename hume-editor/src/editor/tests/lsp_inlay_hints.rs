// Inlay-hint rendering: the `update_inlay_hint_providers`
// write side that feeds the new `InlineDecorationProvider` (INLINE-kind
// `DecorationSource`) from the `decorations.inlay_hints` store.
//
// Every test here goes through `Editor::open(None, std::sync::Arc::new(|| {}))` (not `editor_from`'s bare
// `Pane::new`) — `InlineDecorationProvider` is only registered by `build_pane`, same
// reasoning as `lsp_render.rs`. Hints are injected directly via
// `ed.state.config.decorations.set_inlay_hints` (bypassing `set-inlay-hints!`'s wire
// position/UTF-16 decoding, already covered by `lsp_decorations.rs`) since
// these tests are about the render path, not the Steel/wire boundary.

use super::*;
use crate::editor::decorations::InlayHintEntry;
use hume_engine::pipeline::RenderContext;
use hume_grid::Rect;

#[test]
fn after_hint_renders_dimmed_immediately_after_its_char() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![InlayHintEntry {
            pos: 4, // the 'x'
            text: ": i32".to_string(),
            before: false,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn before_hint_renders_immediately_before_its_char() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![InlayHintEntry {
            pos: 8, // the '5'
            text: "n: ".to_string(),
            before: true,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

/// `prepare_frame` must sync `update_inlay_hint_providers` *before* it
/// scrolls: the scroll step and `content_pos` both build a `RowMap` off the
/// same pane provider Arc `update_inlay_hint_providers` writes, so if scroll
/// ran first it would size line 0's block without the hint (1 row) while
/// `content_pos` — built fresh right after `prepare_frame` returns, as
/// production code does for the terminal caret — sees the hint already
/// written (2 rows) and disagrees about which absolute row the cursor is on.
///
/// Wrap width 3, a `before` hint "HHH" splices in right before line 0's only
/// char ("x") — a mid-line insert, so unlike a trailing/end-of-line insert it
/// participates in wrapping. With the hint, line 0 wraps to 2 rows (`HHH` /
/// `x`), pushing line 2 ("b", the cursor) to absolute row 3. Viewport height
/// 3 content rows (rect height 4, one row reserved for the statusline),
/// scrolloff 0: without the hint, the cursor's row (2) is already the last
/// visible row, so a scroll step that doesn't see the hint decides nothing
/// needs to move.
#[test]
fn hint_arriving_this_frame_is_visible_to_the_scroll_step_that_places_the_cursor() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.state.settings.scrolloff = 0;
    type_text(&mut ed, "x\na\nb");
    let bid = ed.focused_buffer_id();
    ed.set_current_selections(hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(4), // 'b', line 2
    ));
    let pid = ed.state.focused_pane_id;
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 3 }),
        saved: None,
    });
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![InlayHintEntry {
            pos: 0, // the 'x'
            text: "HHH".to_string(),
            before: true, // mid-line insert, so it participates in wrapping
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(10, 4);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let vp = ed.view.panes[pid].viewport.clone();
    let cursor_char = ed.current_selections().primary().head();
    let mut scratch = hume_engine::format::FormatScratch::new();
    let mut rm = crate::editor::commands::pane_row_map(
        ed.doc(),
        &ed.state.settings,
        &ed.view.panes[pid],
        &mut scratch,
    );
    assert_eq!(
        crate::editor::cursor::content_pos(&vp, &mut rm, cursor_char),
        Some((0, 2)),
        "scroll must have already accounted for the hint's extra wrap row, \
         placing the cursor at the last visible row rather than leaving it \
         unplaceable off the bottom"
    );
}

#[test]
fn hint_after_an_emoji_lands_on_the_correct_byte_offset() {
    // "🎉" is 1 char, 4 UTF-8 bytes. An 'after' hint at char index 0 (the
    // emoji itself) must splice in right after its 4 bytes, not after 1
    // byte — proving the write side converts by rope char-to-byte, not by
    // treating `pos` as already a byte count.
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "🎉party");
    let bid = ed.focused_buffer_id();
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![InlayHintEntry {
            pos: 0,
            text: "<HINT>".to_string(),
            before: false,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn hint_on_a_wrapped_line_pins_current_render_behavior() {
    // Documents the accepted engine divergence (inline inserts wrap at
    // render time but are invisible to scroll/cursor row math) — this test
    // only pins whatever `format_buffer_line` currently does, it does not
    // assert correctness of cursor placement on this line.
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "aaaaaaaaaabbbbbbbbbbccccccccccdddddddddd");
    let bid = ed.focused_buffer_id();
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![InlayHintEntry {
            pos: 10, // right after the 'a' run
            text: "[hint]".to_string(),
            before: false,
        }],
    );
    type_cmd(&mut ed, ":set global wrap-mode=soft");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(20, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 20, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn clearing_the_store_removes_the_hint_next_frame() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![InlayHintEntry {
            pos: 4,
            text: ": i32".to_string(),
            before: false,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focused_pane_id;
    let has_hint_before = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .inlay_hints
        .read()
        .unwrap()
        .values()
        .any(|v| !v.is_empty());
    assert!(has_hint_before, "sanity: hint present before clearing");

    ed.state
        .config
        .decorations
        .set_inlay_hints("test".to_string(), bid, vec![]);
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let has_hint_after = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .inlay_hints
        .read()
        .unwrap()
        .values()
        .any(|v| !v.is_empty());
    assert!(
        !has_hint_after,
        "hint must be gone once the store is cleared"
    );
}

#[test]
fn setting_off_does_not_clear_an_unrelated_sources_hints() {
    // The render bridge (`update_inlay_hint_providers`) is not gated on
    // `lsp.inlay-hints` — that setting belongs to the LSP inlay-hints
    // plugin, which owns clearing *its own* source
    // (`"lsp-inlay-hints"`) on toggle-off via the `on-option-change` hook
    // (see `runtime/plugins/core/lsp/inlay.scm`, and the real-plugin
    // regression coverage in `tests/unix/lsp_inlay_feature.rs`). This test
    // has no scripting host attached, so no hook fires — it's checking the
    // render bridge in isolation: a source with no relation to LSP must
    // render regardless of that setting, on or off.
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.config.decorations.set_inlay_hints(
        "some-other-plugin".to_string(),
        bid,
        vec![InlayHintEntry {
            pos: 4,
            text: ": i32".to_string(),
            before: false,
        }],
    );
    let pid = ed.state.focused_pane_id;
    let has_hint = |ed: &mut Editor, ctx: &mut RenderContext| {
        ed.sync_viewport_dims(40, 8);
        ed.settle();
        ed.prepare_frame(ctx);
        ed.state
            .panes
            .render
            .get(pid)
            .unwrap()
            .inlay_hints
            .read()
            .unwrap()
            .values()
            .any(|v| !v.is_empty())
    };

    let mut ctx = RenderContext::new();
    assert!(
        has_hint(&mut ed, &mut ctx),
        "sanity: an unrelated source's hint renders with the setting at its off default"
    );

    type_cmd(&mut ed, ":set global lsp.inlay-hints=true");
    assert!(
        has_hint(&mut ed, &mut ctx),
        "unrelated source must still render once the LSP setting turns on"
    );

    type_cmd(&mut ed, ":set global lsp.inlay-hints=false");
    assert!(
        has_hint(&mut ed, &mut ctx),
        "turning the LSP inlay-hints setting back off must not clear a \
         source it doesn't own"
    );
}
