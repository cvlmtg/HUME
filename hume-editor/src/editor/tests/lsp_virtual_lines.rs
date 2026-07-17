// Inline diagnostics VirtualLineSource: the
// `update_virtual_line_providers` write side that feeds the new
// `PaneVirtualLines` (`VirtualLineSource`) from the `decorations.virtual_lines`
// store, over the virtual-row-aware scroll/cursor plumbing.
//
// Every test here goes through `Editor::open(None)` (not `editor_from`'s
// bare `Pane::new`) — `PaneVirtualLines` is only registered by `build_pane`,
// same reasoning as `lsp_render.rs`.

use std::path::Path;

use super::*;
use hume_engine::pipeline::RenderContext;
use hume_scripting::ScriptingHost;
use ratatui::layout::Rect;

fn run(ed: &mut Editor, tmp: &Path, source: &str) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(ed, &mut host, source, tmp);
    ed.scripting = Some(host);
}

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
fn virtual_line_renders_after_its_anchor_line() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "let x = 5\nlet y = 10");
    let bid = ed.focused_buffer_id();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (list 0 "^ unused variable" "diagnostic.warning")))))"#,
    );
    type_cmd(&mut ed, ":go");
    let _ = bid;

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn scroll_over_a_virtual_line_pushes_the_next_line_down_correctly() {
    // Snapshot proof (not a numeric row assertion — `cursor::screen_pos`'s
    // internals aren't reachable from this integration-style test module):
    // with a virtual line inserted after line 0, moving the cursor onto
    // line 1 ('bbb') must still show "bbb" directly below the virtual
    // line's own row — the row-counting fix must correctly push line
    // 1's content down by the one stolen row, never overlap or skip it.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "aaa\nbbb\nccc");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (set-virtual-lines! "linter" (current-buffer)
               (list (list 0 "hint")))))"#,
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
    ed.prepare_frame(40, 8, &mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn clearing_the_store_removes_the_virtual_line_next_frame() {
    let mut ed = Editor::open(None).unwrap();
    type_text(&mut ed, "let x = 5");
    let bid = ed.focused_buffer_id();
    ed.state.decorations.set_virtual_lines(
        "linter".to_string(),
        bid,
        vec![crate::editor::decorations::VirtualLineEntry {
            line: 0,
            text: "hint".to_string(),
            scope: None,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);
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
        .decorations
        .set_virtual_lines("linter".to_string(), bid, vec![]);
    ed.prepare_frame(40, 8, &mut ctx);
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
