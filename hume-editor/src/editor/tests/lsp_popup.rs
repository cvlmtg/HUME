// Cursor-anchored popup widget: `show-popup!` /
// `close-popup!`, and the `sync_popup_view` write side that resolves
// geometry (wrap + flip/clamp) fresh every frame from the focused pane's
// current rect.

use std::path::Path;

use super::*;
use hume_engine::pipeline::RenderContext;
use hume_scripting::ScriptingHost;

fn run(ed: &mut Editor, tmp: &Path, source: &str) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(ed, &mut host, source, tmp);
    ed.scripting = Some(host);
}

fn popup_view(ed: &Editor) -> Option<(Vec<String>, u16, u16)> {
    ed.state
        .popup_view
        .read()
        .unwrap()
        .as_ref()
        .map(|s| (s.lines.clone(), s.x, s.y))
}

// ── show-popup! / close-popup! ────────────────────────────────────────────────

#[test]
fn show_popup_populates_the_view_after_a_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello")))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let (lines, _, _) = popup_view(&ed).expect("popup must be showing after a frame");
    assert_eq!(lines, vec!["hello"]);
}

#[test]
fn close_popup_clears_the_view() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello")))
           (define-command! "gone" "" (lambda () (close-popup!)))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);
    assert!(popup_view(&ed).is_some(), "sanity: showing");

    type_cmd(&mut ed, ":gone");
    ed.prepare_frame(80, 25, &mut ctx);
    assert!(
        popup_view(&ed).is_none(),
        "must be cleared after close-popup!"
    );
}

#[test]
fn show_popup_replaces_not_stacks() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "arm-first" "" (lambda () (show-popup! "first")))
           (define-command! "arm-second" "" (lambda () (show-popup! "second")))"#,
    );
    type_cmd(&mut ed, ":arm-first");
    type_cmd(&mut ed, ":arm-second");
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let (lines, _, _) = popup_view(&ed).unwrap();
    assert_eq!(lines, vec!["second"]);
}

#[test]
fn show_popup_rejects_a_non_cursor_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hi" #:anchor 'bottom)))"#,
    );
    type_cmd(&mut ed, ":go");
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("anchor") || msg.to_lowercase().contains("error"),
        "expected an error about the unsupported anchor, got {msg:?}"
    );
}

// ── Geometry: wrap width / flip / clamp against a real pane ─────────────────

#[test]
fn popup_wraps_to_the_pane_width_and_anchors_below_the_cursor() {
    let tmp = tempfile::tempdir().unwrap();
    // Cursor at column 0, row 0 — plenty of room below in a 25-row terminal.
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (show-popup! "one two three four five six seven eight nine ten")))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.prepare_frame(20, 25, &mut ctx);

    let (lines, x, y) = popup_view(&ed).unwrap();
    assert!(
        lines.len() > 1,
        "text wider than the pane must wrap to multiple lines"
    );
    assert!(
        lines
            .iter()
            .all(|l| unicode_width::UnicodeWidthStr::width(l.as_str()) <= 16),
        "no line may exceed min(60, pane_width - 4): {lines:?}"
    );
    assert!(
        y >= 1,
        "popup must anchor below the cursor row (row 0), not above it"
    );
    assert_eq!(x, 0, "anchor column matches the cursor's column");
}

// ── Scrollable popup (`#:scroll`) dismissal + Ctrl+u/Ctrl+d ─────────────────

#[test]
fn ctrl_d_and_ctrl_u_scroll_a_scrollable_popup_without_touching_the_buffer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    let tall = (0..30)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "go" "" (lambda () (show-popup! "{tall}" #:scroll #t)))"#),
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let before = state(&ed);
    assert_eq!(
        ed.state.popup.as_ref().expect("shown").scroll,
        0,
        "sanity: starts unscrolled"
    );

    ed.feed_key(key_ctrl('d'));
    ed.prepare_frame(80, 25, &mut ctx);
    let scroll_after_down = ed
        .state
        .popup
        .as_ref()
        .expect("Ctrl+d must not close a scrollable popup")
        .scroll;
    assert!(
        scroll_after_down > 0,
        "Ctrl+d must scroll a scrollable popup's content forward"
    );
    assert_eq!(
        state(&ed),
        before,
        "Ctrl+d must scroll the popup, not the buffer, while a scrollable popup is open"
    );

    ed.feed_key(key_ctrl('u'));
    ed.prepare_frame(80, 25, &mut ctx);
    let scroll_after_up = ed
        .state
        .popup
        .as_ref()
        .expect("Ctrl+u must not close a scrollable popup")
        .scroll;
    assert!(
        scroll_after_up < scroll_after_down,
        "Ctrl+u must scroll a scrollable popup's content back"
    );
}

#[test]
fn any_other_key_closes_a_scrollable_popup_and_still_dispatches() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello" #:scroll #t)))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);
    assert!(popup_view(&ed).is_some(), "sanity: showing");
    let head_before = ed.current_selections().primary().head();

    // 'l' both closes the popup and still moves the cursor right — a stray
    // key on a scrollable popup is dismiss-and-fall-through, not
    // dismiss-and-swallow.
    ed.feed_key(key('l'));
    assert!(
        ed.state.popup.is_none(),
        "any non-scroll key must close a scrollable popup"
    );
    assert_eq!(
        ed.current_selections().primary().head(),
        head_before + 1,
        "the closing key ('l') must still execute its normal motion"
    );
}

#[test]
fn ctrl_d_on_a_non_scroll_popup_still_scrolls_the_buffer() {
    // Regression guard: a popup without `#:scroll` (hover/sighelp today,
    // or the diagnostic overlay before its own `#:dismiss-on-key` clear)
    // must leave Ctrl+d/Ctrl+u to their ordinary half-page-scroll binding.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello")))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let before = state(&ed);
    ed.feed_key(key_ctrl('d'));

    assert!(
        matches!(
            ed.state.popup.as_ref().map(|p| &p.dismiss),
            Some(crate::ui::popup::PopupDismiss::ModeChange)
        ),
        "a plain popup must be untouched by Ctrl+d"
    );
    assert_ne!(
        state(&ed),
        before,
        "Ctrl+d must still run half-page-down on the buffer when the open popup isn't scrollable"
    );
}

#[test]
fn scrollable_popup_paints_its_scrolled_window() {
    // Appearance lock: the painted rows actually shift after Ctrl+d, not
    // just the underlying `scroll` field (a regression in `draw_menu_box`'s
    // windowing wouldn't be caught by the data-only assertions above).
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    let tall = (0..20)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "go" "" (lambda () (show-popup! "{tall}" #:scroll #t)))"#),
    );
    type_cmd(&mut ed, ":go");

    use ratatui::layout::Rect;
    let rect = Rect::new(0, 0, 30, 15);
    // Render once to resolve `popup_view` geometry before scrolling —
    // `scroll_popup` reads the previous frame's resolved height, same as
    // real interactive use (a keystroke always follows at least one paint).
    let _ = render_snapshot::render_to_styled_string(&mut ed, rect);
    ed.feed_key(key_ctrl('d'));
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn popup_never_paints_outside_the_pane_rect() {
    // A snapshot-level end-to-end check: render into a small terminal and
    // confirm every non-space cell the popup could have touched stays
    // within the pane rows (no bleed into the statusline row).
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.feed_key(key('i'));
    for ch in "hello".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hover text")))"#,
    );
    type_cmd(&mut ed, ":go");

    use ratatui::layout::Rect;
    let rect = Rect::new(0, 0, 30, 8);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
