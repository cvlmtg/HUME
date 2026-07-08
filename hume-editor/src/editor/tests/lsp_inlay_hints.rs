// U9 (docs/lsp/step-3.md) — inlay-hint rendering: the `update_inlay_hint_providers`
// write side that feeds the new `InlayHintProvider` (`InlineDecoration`) from
// B5's `decorations.inlay_hints` store.
//
// Every test here goes through `Editor::open(None)` (not `editor_from`'s bare
// `Pane::new`) — `InlayHintProvider` is only registered by `build_pane`, same
// reasoning as U1's `lsp_render.rs`. Hints are injected directly via
// `ed.state.decorations.set_inlay_hints` (bypassing `set-inlay-hints!`'s wire
// position/UTF-16 decoding, already covered by `lsp_decorations.rs`) since
// these tests are about the render path, not the Steel/wire boundary.

use super::*;
use crate::editor::decorations::InlayHintEntry;
use hume_engine::pipeline::RenderContext;
use ratatui::layout::Rect;

fn type_text(ed: &mut Editor, text: &str) {
    ed.feed_key(key('i'));
    for ch in text.chars() {
        if ch == '\n' {
            ed.feed_key(key_enter());
        } else {
            ed.feed_key(key(ch));
        }
    }
    ed.feed_key(key_esc());
}

#[test]
fn after_hint_renders_dimmed_immediately_after_its_char() {
    let mut ed = Editor::open(None).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.decorations.set_inlay_hints(
        bid,
        vec![InlayHintEntry {
            pos: 4, // the 'x'
            text: ": i32".to_string(),
            before: false,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn before_hint_renders_immediately_before_its_char() {
    let mut ed = Editor::open(None).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.decorations.set_inlay_hints(
        bid,
        vec![InlayHintEntry {
            pos: 8, // the '5'
            text: "n: ".to_string(),
            before: true,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn hint_after_an_emoji_lands_on_the_correct_byte_offset() {
    // "🎉" is 1 char, 4 UTF-8 bytes. An 'after' hint at char index 0 (the
    // emoji itself) must splice in right after its 4 bytes, not after 1
    // byte — proving the write side converts by rope char-to-byte, not by
    // treating `pos` as already a byte count.
    let mut ed = Editor::open(None).unwrap();
    type_text(&mut ed, "🎉party");
    let bid = ed.focused_buffer_id();
    ed.state.decorations.set_inlay_hints(
        bid,
        vec![InlayHintEntry {
            pos: 0,
            text: "<HINT>".to_string(),
            before: false,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn hint_on_a_wrapped_line_pins_current_render_behavior() {
    // Documents the accepted engine divergence (inline inserts wrap at
    // render time but are invisible to scroll/cursor row math) — this test
    // only pins whatever `format_buffer_line` currently does, it does not
    // assert correctness of cursor placement on this line.
    let mut ed = Editor::open(None).unwrap();
    type_text(&mut ed, "aaaaaaaaaabbbbbbbbbbccccccccccdddddddddd");
    let bid = ed.focused_buffer_id();
    ed.state.decorations.set_inlay_hints(
        bid,
        vec![InlayHintEntry {
            pos: 10, // right after the 'a' run
            text: "[hint]".to_string(),
            before: false,
        }],
    );
    type_cmd(&mut ed, ":set global wrap-mode=soft");

    let mut ctx = RenderContext::new();
    ed.prepare_frame(20, 8, &mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 20, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn clearing_the_store_removes_the_hint_next_frame() {
    let mut ed = Editor::open(None).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.decorations.set_inlay_hints(
        bid,
        vec![InlayHintEntry {
            pos: 4,
            text: ": i32".to_string(),
            before: false,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);
    let pid = ed.state.focused_pane_id;
    let has_hint_before = ed
        .state
        .panes
        .inlay_hints
        .get(pid)
        .unwrap()
        .read()
        .unwrap()
        .values()
        .any(|v| !v.is_empty());
    assert!(has_hint_before, "sanity: hint present before clearing");

    ed.state.decorations.set_inlay_hints(bid, vec![]);
    ed.prepare_frame(40, 8, &mut ctx);
    let has_hint_after = ed
        .state
        .panes
        .inlay_hints
        .get(pid)
        .unwrap()
        .read()
        .unwrap()
        .values()
        .any(|v| !v.is_empty());
    assert!(!has_hint_after, "hint must be gone once the store is cleared");
}

#[test]
fn setting_off_renders_nothing_even_with_hints_in_the_store() {
    let mut ed = Editor::open(None).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.decorations.set_inlay_hints(
        bid,
        vec![InlayHintEntry {
            pos: 4,
            text: ": i32".to_string(),
            before: false,
        }],
    );
    type_cmd(&mut ed, ":set global lsp.inlay-hints=false");

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);
    let pid = ed.state.focused_pane_id;
    let has_hint = ed
        .state
        .panes
        .inlay_hints
        .get(pid)
        .unwrap()
        .read()
        .unwrap()
        .values()
        .any(|v| !v.is_empty());
    assert!(!has_hint, "setting off must clear the provider map, not just skip refreshing it");
}
