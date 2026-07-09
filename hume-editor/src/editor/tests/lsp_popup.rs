// Cursor-anchored popup widget: `show-popup!` /
// `close-popup!`, and the `sync_popup_view` write side that resolves
// geometry (wrap + flip/clamp) fresh every frame from the focused pane's
// current rect.

use std::path::Path;

use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_engine::pipeline::RenderContext;
use hume_scripting::ScriptingHost;

fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

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

#[test]
fn popup_never_paints_outside_the_pane_rect() {
    // A snapshot-level end-to-end check: render into a small terminal and
    // confirm every non-space cell the popup could have touched stays
    // within the pane rows (no bleed into the statusline row).
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None).unwrap();
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
