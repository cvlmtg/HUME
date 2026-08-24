// Cursor-anchored popup widget: `show-popup!` /
// `close-popup!`, and the `sync_popup_view` write side that resolves
// geometry (wrap + flip/clamp) fresh every frame from the focused pane's
// current rect.

use std::path::Path;
use std::sync::Arc;

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
        .map(|s| ((*s.lines).clone(), s.x, s.y))
}

/// The `Arc` handle itself, not a deref-cloned copy — for `Arc::ptr_eq`
/// identity checks that pin `PopupModel::resolved`'s per-`max_width` cache.
fn popup_view_lines_arc(ed: &Editor) -> Option<Arc<Vec<String>>> {
    ed.state
        .popup_view
        .read()
        .unwrap()
        .as_ref()
        .map(|s| Arc::clone(&s.lines))
}

// ── show-popup! / close-popup! ────────────────────────────────────────────────

#[test]
fn show_popup_populates_the_view_after_a_frame() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello")))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let (lines, _, _) = popup_view(&ed).expect("popup must be showing after a frame");
    assert_eq!(lines, vec!["hello"]);
}

#[test]
fn close_popup_clears_the_view() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello")))
           (define-command! "gone" "" (lambda () (close-popup!)))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(popup_view(&ed).is_some(), "sanity: showing");

    type_cmd(&mut ed, ":gone");
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(
        popup_view(&ed).is_none(),
        "must be cleared after close-popup!"
    );
}

#[test]
fn show_popup_replaces_not_stacks() {
    let tmp = safe_tempdir();
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
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let (lines, _, _) = popup_view(&ed).unwrap();
    assert_eq!(lines, vec!["second"]);
}

#[test]
fn show_popup_rejects_an_unknown_anchor() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hi" #:anchor 'top)))"#,
    );
    type_cmd(&mut ed, ":go");
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("anchor") || msg.to_lowercase().contains("error"),
        "expected an error about the unsupported anchor, got {msg:?}"
    );
}

// ── Docked layout (`#:anchor 'bottom`) ──────────────────────────────────────

fn popup_band_lines(ed: &Editor) -> Option<Vec<String>> {
    ed.state
        .popup_band_view
        .read()
        .unwrap()
        .as_ref()
        .map(|s| (*s.lines).clone())
}

#[test]
fn docked_popup_resolves_into_the_band_view_not_the_cursor_overlay() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello" #:anchor 'bottom)))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(popup_band_lines(&ed), Some(vec!["hello".to_string()]));
    assert!(
        popup_view(&ed).is_none(),
        "a docked popup must not populate the cursor-anchored overlay view"
    );
}

#[test]
fn close_popup_clears_the_band_view_too() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello" #:anchor 'bottom)))
           (define-command! "gone" "" (lambda () (close-popup!)))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(popup_band_lines(&ed).is_some(), "sanity: showing");

    type_cmd(&mut ed, ":gone");
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(
        popup_band_lines(&ed).is_none(),
        "must be cleared after close-popup!"
    );
}

#[test]
fn ctrl_d_and_ctrl_u_scroll_a_docked_popup_without_touching_the_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    let tall = (0..30)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda () (show-popup! "{tall}" #:kind 'scrollable #:anchor 'bottom)))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    // A 10-row terminal caps the band at height/2 = 5 rows (3 content rows
    // after the 2-cell frame) — well under the 30 lines, so scrolling has
    // somewhere to go.
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 10);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let before = state(&ed);
    assert_eq!(ed.state.config.popup.as_ref().expect("shown").scroll, 0);

    ed.feed_key(key_ctrl('d'));
    ed.sync_viewport_dims(80, 10);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let scroll_after_down = ed
        .state
        .config
        .popup
        .as_ref()
        .expect("Ctrl+d must not close it")
        .scroll;
    assert!(
        scroll_after_down > 0,
        "Ctrl+d must scroll a docked popup's content forward"
    );
    assert_eq!(
        state(&ed),
        before,
        "Ctrl+d must scroll the popup, not the buffer"
    );

    ed.feed_key(key_ctrl('u'));
    ed.sync_viewport_dims(80, 10);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let scroll_after_up = ed
        .state
        .config
        .popup
        .as_ref()
        .expect("Ctrl+u must not close it")
        .scroll;
    assert!(
        scroll_after_up < scroll_after_down,
        "Ctrl+u must scroll a docked popup's content back"
    );
}

#[test]
fn any_other_key_closes_a_docked_popup_and_still_dispatches() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello" #:kind 'scrollable #:anchor 'bottom)))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(popup_band_lines(&ed).is_some(), "sanity: showing");
    let head_before = ed.current_selections().primary().head();

    ed.feed_key(key('l'));
    assert!(
        ed.state.config.popup.is_none(),
        "any non-scroll key must close a docked popup"
    );
    assert_eq!(
        ed.current_selections().primary().head(),
        head_before + 1,
        "the closing key ('l') must still execute its normal motion"
    );
}

#[test]
fn dismiss_key_repaints_the_rows_a_docked_popup_vacated_on_the_very_next_frame() {
    use ratatui::layout::Rect;
    let tmp = safe_tempdir();
    // `editor_from` (`Editor::for_testing`) never registers `bottom_bands` —
    // only `Editor::open`'s real startup path does — so a docked popup there
    // never actually shrinks `pane_area`. This test asserts on that
    // geometry, so it needs the real registration.
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let tall = (0..30)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda () (show-popup! "{tall}" #:kind 'scrollable #:anchor 'bottom)))"#
        ),
    );
    type_cmd(&mut ed, ":go");

    let rect = Rect::new(0, 0, 80, 25);
    let _ = ed.render_to_buf(rect); // frame 1: band visible
    assert!(popup_band_lines(&ed).is_some(), "sanity: band showing");
    let band_top = ed.view.pane_area(rect).height;
    assert!(
        band_top < rect.height - 1,
        "sanity: band must shrink the pane"
    );

    ed.feed_key(key('l')); // any non-scroll key dismisses a docked popup
    assert!(
        ed.state.config.popup.is_none(),
        "sanity: model closed by the dismiss key"
    );

    let pid = ed.state.focused_pane_id;
    let buf = ed.render_to_buf(rect); // frame 2: the close frame
    assert_eq!(
        ed.view.panes[pid].viewport.height,
        ed.view.pane_area(rect).height,
        "viewport height must match the pane rect this frame's render painted into"
    );
    for y in band_top..rect.height - 1 {
        // rect.height - 1 excludes the statusline row.
        assert!(
            (0..rect.width).any(|x| buf[(x, y)].symbol() != " "),
            "row {y} (vacated by the closed band) must be repainted this frame, not left blank"
        );
    }
}

#[test]
fn settle_driven_close_repaints_the_rows_a_docked_popup_vacated_on_the_very_next_frame() {
    use ratatui::layout::Rect;
    let tmp = safe_tempdir();
    // See the sibling test above: needs `Editor::open`'s real `bottom_bands`
    // registration for the pane-shrinking geometry this test asserts on.
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let tall = (0..30)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda () (show-popup! "{tall}" #:kind 'scrollable #:anchor 'bottom)))"#
        ),
    );
    type_cmd(&mut ed, ":go");

    let rect = Rect::new(0, 0, 80, 25);
    let _ = ed.render_to_buf(rect); // frame 1: band visible
    assert!(popup_band_lines(&ed).is_some(), "sanity: band showing");
    let band_top = ed.view.pane_area(rect).height;
    assert!(
        band_top < rect.height - 1,
        "sanity: band must shrink the pane"
    );

    // Register the real `lib.scm:38` hook only now — done as a separate
    // `run()` (registering earlier would have it fire on the Command-mode
    // entry/exit `:go`'s own dispatch triggers, closing the popup before
    // frame 1 finished, since a docked popup is otherwise indistinguishable
    // from any other mode transition to this unconditional hook).
    run(
        &mut ed,
        tmp.path(),
        r#"(register-hook! 'on-mode-change (lambda (old new) (close-popup!)))"#,
    );

    // `EditorState::set_mode` only queues `OnModeChange` (the same funnel a
    // programmatic mode change — LSP callback, plugin command — goes
    // through); unlike a keypress it never runs through `handle_key`'s
    // any-key-closes-a-docked-popup intercept, so the popup is still open
    // here. The `on-mode-change` hook (mirroring `lib.scm`'s real one) only
    // runs when `render_to_buf`'s `settle()` drains the queued event —
    // reproducing the settle-time close window a real hook uses.
    ed.state.set_mode(Mode::Insert);
    assert!(
        ed.state.config.popup.is_some(),
        "sanity: popup still open before settle drains the queued hook"
    );

    let pid = ed.state.focused_pane_id;
    let buf = ed.render_to_buf(rect); // frame 2: settle() drains the hook, which closes the popup
    assert!(
        ed.state.config.popup.is_none(),
        "sanity: hook must have closed the popup during settle"
    );
    assert_eq!(
        ed.view.panes[pid].viewport.height,
        ed.view.pane_area(rect).height,
        "viewport height must match the pane rect this frame's render painted into"
    );
    for y in band_top..rect.height - 1 {
        assert!(
            (0..rect.width).any(|x| buf[(x, y)].symbol() != " "),
            "row {y} (vacated by the closed band) must be repainted this frame, not left blank"
        );
    }
}

#[test]
fn docked_popup_renders_as_a_band_above_the_statusline_and_shrinks_the_pane() {
    // Appearance + layout lock: the docked popup must actually reserve
    // chrome space (pane shrinks), not float over content like the cursor
    // layout.
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "docked hover text" #:anchor 'bottom)))"#,
    );
    type_cmd(&mut ed, ":go");

    use ratatui::layout::Rect;
    let rect = Rect::new(0, 0, 40, 10);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

// ── Geometry: wrap width / flip / clamp against a real pane ─────────────────

#[test]
fn popup_wraps_to_the_pane_width_and_anchors_below_the_cursor() {
    let tmp = safe_tempdir();
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
    ed.sync_viewport_dims(20, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

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
fn wrap_is_cached_per_width_and_invalidated_only_when_width_changes() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (show-popup! "one two three four five six seven eight nine ten")))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(20, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let first = popup_view_lines_arc(&ed).expect("popup must be showing after a frame");

    ed.sync_viewport_dims(20, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let second = popup_view_lines_arc(&ed).expect("popup must still be showing");
    assert!(
        Arc::ptr_eq(&first, &second),
        "wrap must not be recomputed across frames at an unchanged max_width"
    );

    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let third = popup_view_lines_arc(&ed).expect("popup must still be showing");
    assert!(
        !Arc::ptr_eq(&second, &third),
        "wrap must be recomputed once max_width actually changes"
    );
}

// ── Scrollable popup (`#:kind 'scrollable`) dismissal + Ctrl+u/Ctrl+d ───────

#[test]
fn ctrl_d_and_ctrl_u_scroll_a_scrollable_popup_without_touching_the_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    let tall = (0..30)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda () (show-popup! "{tall}" #:kind 'scrollable)))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let before = state(&ed);
    assert_eq!(
        ed.state.config.popup.as_ref().expect("shown").scroll,
        0,
        "sanity: starts unscrolled"
    );

    ed.feed_key(key_ctrl('d'));
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let scroll_after_down = ed
        .state
        .config
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
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let scroll_after_up = ed
        .state
        .config
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
fn ctrl_u_clamps_a_stale_scroll_after_the_window_grows_between_frames() {
    // Regression: `PopupModel::scroll` is clamped for *rendering* every
    // frame, but that clamp writes only into the view copy, never back into
    // the model. If the popup's visible window grows without a scroll key
    // touching the model (e.g. the terminal resizes taller), the model can
    // hold a scroll value now far beyond the shrunk `max_scroll`. Fail
    // oracle: subtracting from that stale value directly, without first
    // clamping it to the current `max_scroll`, could still land above it —
    // visibly a no-op on the first Ctrl+u press.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    let tall = (0..40)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda () (show-popup! "{tall}" #:kind 'scrollable)))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    // Force a stale scroll far beyond what a much taller frame's window
    // will allow, standing in for a scroll set before the terminal grew.
    ed.state.config.popup.as_mut().expect("shown").scroll = 30;

    // Grow the frame: more visible rows, so `max_scroll` shrinks well below
    // the stale value set above.
    ed.sync_viewport_dims(80, 80);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let max_scroll_before_key = {
        let guard = ed.state.popup_view.read().unwrap();
        let view = guard.as_ref().expect("popup still open");
        let inner_h = view.outer_h.saturating_sub(2) as usize;
        view.lines.len().saturating_sub(inner_h)
    };
    assert!(
        max_scroll_before_key < 30,
        "sanity: the taller frame must shrink max_scroll below the stale value"
    );

    ed.feed_key(key_ctrl('u'));
    ed.sync_viewport_dims(80, 80);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let scroll_after_up = ed
        .state
        .config
        .popup
        .as_ref()
        .expect("Ctrl+u must not close a scrollable popup")
        .scroll;
    assert!(
        scroll_after_up <= max_scroll_before_key,
        "Ctrl+u must clamp a stale model scroll to the current window before \
         subtracting, not just after — got {scroll_after_up}, current max was \
         {max_scroll_before_key}"
    );
}

#[test]
fn any_other_key_closes_a_scrollable_popup_and_still_dispatches() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello" #:kind 'scrollable)))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(popup_view(&ed).is_some(), "sanity: showing");
    let head_before = ed.current_selections().primary().head();

    // 'l' both closes the popup and still moves the cursor right — a stray
    // key on a scrollable popup is dismiss-and-fall-through, not
    // dismiss-and-swallow.
    ed.feed_key(key('l'));
    assert!(
        ed.state.config.popup.is_none(),
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
    // Regression guard: a plain popup (`#:kind` omitted or `'sticky` —
    // hover/sighelp today, or the diagnostic overlay before its own
    // `'transient` clear) must leave Ctrl+d/Ctrl+u to their ordinary
    // half-page-scroll binding.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello")))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let before = state(&ed);
    ed.feed_key(key_ctrl('d'));

    assert!(
        matches!(
            ed.state.config.popup.as_ref().map(|p| p.kind),
            Some(hume_scripting::host::PopupKind::Sticky)
        ),
        "a plain popup must be untouched by Ctrl+d"
    );
    assert_ne!(
        state(&ed),
        before,
        "Ctrl+d must still run half-page-down on the buffer when the open popup isn't scrollable"
    );
}

// ── Mouse dismissal ──────────────────────────────────────────────────────

#[test]
fn a_mouse_wheel_closes_a_scrollable_popup_and_still_scrolls() {
    // Buffer taller than the viewport, so the wheel tick genuinely has
    // somewhere to scroll — distinguishes "dismissed" from "dismissed and
    // the event's own effect was swallowed along with it".
    let tmp = safe_tempdir();
    let mut lines = String::from("-[x]>line0\n");
    for i in 1..40 {
        lines.push_str(&format!("line{i}\n"));
    }
    let mut ed = editor_from(&lines);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello" #:kind 'scrollable)))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(popup_view(&ed).is_some(), "sanity: showing");
    let top_before = ed.viewport().top_line;

    ed.handle_input(mouse_wheel(true));

    assert!(
        ed.state.config.popup.is_none(),
        "a mouse wheel tick must close a scrollable popup"
    );
    assert_eq!(
        ed.viewport().top_line,
        top_before + ed.state.settings.mouse_scroll_lines,
        "the wheel tick must still scroll the buffer in the same event"
    );
}

#[test]
fn a_mouse_click_closes_a_scrollable_popup() {
    // Normal-mode click: no mode change happens, so this can't pass via the
    // `on-mode-change` hook masking the missing mouse-side dismissal.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello" #:kind 'scrollable)))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(popup_view(&ed).is_some(), "sanity: showing");

    ed.handle_input(mouse_left_down(3, 0));

    assert!(
        ed.state.config.popup.is_none(),
        "a mouse click must close a scrollable popup"
    );
    assert_eq!(
        ed.current_selections().primary().head(),
        3,
        "the click must still move the cursor to the clicked char"
    );
}

#[test]
fn a_sticky_popup_survives_mouse_input() {
    // Regression guard, not a red/green case: signature help's default
    // `'sticky` popup must stay untouched by mouse input, same as by keys.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (show-popup! "hello")))"#,
    );
    type_cmd(&mut ed, ":go");
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(
        matches!(
            ed.state.config.popup.as_ref().map(|p| p.kind),
            Some(hume_scripting::host::PopupKind::Sticky)
        ),
        "sanity: sticky by default"
    );

    ed.handle_input(mouse_wheel(true));

    assert!(
        matches!(
            ed.state.config.popup.as_ref().map(|p| p.kind),
            Some(hume_scripting::host::PopupKind::Sticky)
        ),
        "a sticky popup must be untouched by mouse input"
    );
}

#[test]
fn scrollable_popup_paints_its_scrolled_window() {
    // Appearance lock: the painted rows actually shift after Ctrl+d, not
    // just the underlying `scroll` field (a regression in `draw_menu_box`'s
    // windowing wouldn't be caught by the data-only assertions above).
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    let tall = (0..20)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda () (show-popup! "{tall}" #:kind 'scrollable)))"#
        ),
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
    let tmp = safe_tempdir();
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
