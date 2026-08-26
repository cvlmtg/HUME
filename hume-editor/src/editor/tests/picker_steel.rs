// The Steel surface for the fuzzy picker:
// (picker! items on-select #:prompt "…") / (picker-push! token items) /
// (picker-close!). The Rust store/widget/key-handling underneath is
// covered by `tests/picker.rs`, which builds sessions directly — this
// file exercises the builtins themselves end to end through real Steel
// source, mirroring `lsp_drawer.rs`'s `run`/`arm_*` pattern.
//
// The picker is full-modal: once one is
// open, `handle_picker_key` intercepts every key ahead of mode dispatch, so
// a raw `:command` typed via `type_cmd` never reaches the minibuffer — it's
// swallowed as picker query input instead. Tests that need to invoke a
// *second* named command while a picker is already open (pushing into it,
// closing it, replacing it) go through `execute_keymap_command` instead,
// bypassing key routing entirely — the same tool `sync_dispatch.rs` uses to
// invoke a named command directly. This also matches the realistic trigger
// for those scenarios: a Steel-level call (async callback, a bound
// command), not a keystroke a full-modal picker would eat.

use std::path::Path;

use super::*;
use crate::editor::dispatch::ArgSource;
use hume_engine::pipeline::RenderContext;
use hume_scripting::ScriptingHost;
use steel::rvals::SteelVal;

fn run(ed: &mut Editor, tmp: &Path, source: &str) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(ed, &mut host, source, tmp);
    ed.scripting = Some(host);
}

fn call(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false, ArgSource::Keymap);
}

// ── picker! opens a session and returns its token ─────────────────────────

#[test]
fn picker_bang_opens_session_and_returns_its_token() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (log! 'info (to-string (picker! (list (cons "one" "p1") (cons "two" "p2"))
               (lambda (x) (log! 'info (to-string x)))
               #:prompt "sel: ")))))"#,
    );
    type_cmd(&mut ed, ":go");

    let session = ed
        .state
        .config
        .picker
        .as_ref()
        .expect("picker! must open a session");
    assert_eq!(session.total_len(), 2);
    assert_eq!(session.prompt(), "sel: ");

    let logged_token: u64 = ed
        .state
        .status_msg
        .clone()
        .unwrap()
        .parse()
        .expect("picker! must return the token as an integer");
    assert_eq!(logged_token, session.token());
}

// ── End-to-end: open, type a query, accept, keep interacting (LESSONS L4) ──

#[test]
fn end_to_end_accept_fires_payload_then_normal_editing_resumes() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1") (cons "two" "p2"))
               (lambda (x) (log! 'info (to-string x))))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    ed.feed_key(key('t'));
    ed.feed_key(key('w'));
    ed.feed_key(key_enter());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "p2");
    assert!(ed.state.config.picker.is_none());

    // Don't stop at the terminal action — keep interacting
    // and confirm ordinary editing resumes with no further callback fire.
    ed.state.status_msg = None;
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    ed.settle();

    assert!(
        ed.state.status_msg.is_none(),
        "no further callback should fire after the picker already closed"
    );
    let bid = ed.focused_buffer_id();
    let text = ed.state.buffers.get(bid).text().to_string();
    assert!(
        text.contains('X'),
        "typing after accept must edit the buffer, got {text:?}"
    );
}

// ── picker-push!: applies on a matching token, #f otherwise ────────────────

#[test]
fn picker_push_bang_applies_matching_token_and_rejects_stale_or_no_picker() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! (list (cons "one" "p1")) (lambda (x) (log! 'info (to-string x)))))))
        (define-command! "push-real" "" (lambda ()
          (log! 'info (to-string (picker-push! tok (list (cons "two" "p2")))))))
        (define-command! "push-stale" "" (lambda ()
          (log! 'info (to-string (picker-push! (+ tok 1) (list (cons "three" "p3")))))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.state.config.picker.as_ref().unwrap().total_len(), 1);

    call(&mut ed, "push-real");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#true");
    assert_eq!(ed.state.config.picker.as_ref().unwrap().total_len(), 2);

    call(&mut ed, "push-stale");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        2,
        "a stale-token push must not apply"
    );

    ed.feed_key(key_esc());
    ed.settle();
    assert!(ed.state.config.picker.is_none());

    call(&mut ed, "push-real");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "#false",
        "pushing after the picker closed must be a silent #f, not an error"
    );
    assert!(ed.state.config.picker.is_none());
}

// ── picker! over an already-open picker fires the old callback once ────────

#[test]
fn opening_a_second_picker_fires_the_first_callback_with_false_exactly_once() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go-a" "" (lambda ()
             (picker! (list (cons "a-item" "pa")) (lambda (x) (log! 'info (to-string "A:" x))))))
           (define-command! "go-b" "" (lambda ()
             (picker! (list (cons "b-item" "pb")) (lambda (x) (log! 'info (to-string "B:" x))))))"#,
    );
    type_cmd(&mut ed, ":go-a");
    call(&mut ed, "go-b");

    assert_eq!(
        pending_calls(&ed).len(),
        1,
        "replacing the open picker must queue exactly one callback (the old one, with #f)"
    );
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "A: #false");

    ed.state.status_msg = None;
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "B: pb");
    assert!(ed.state.config.picker.is_none());
}

// ── picker-close!: fires #f exactly once, idempotent, keeps L4 discipline ──

#[test]
fn picker_close_bang_fires_false_once_and_is_idempotent() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1")) (lambda (x) (log! 'info (to-string x))))))
           (define-command! "close-it" "" (lambda () (picker-close!)))"#,
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.config.picker.is_some());

    call(&mut ed, "close-it");
    assert!(ed.state.config.picker.is_none());
    assert_eq!(pending_calls(&ed).len(), 1);

    call(&mut ed, "close-it");
    assert_eq!(
        pending_calls(&ed).len(),
        1,
        "closing an already-closed picker must not queue a second callback"
    );

    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");

    // Keep interacting past the terminal action.
    ed.state.status_msg = None;
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc());
    ed.settle();

    assert!(
        ed.state.status_msg.is_none(),
        "no further callback must fire"
    );
    let bid = ed.focused_buffer_id();
    let text = ed.state.buffers.get(bid).text().to_string();
    assert!(
        text.contains('Z'),
        "keys after close must behave as plain input"
    );
}

// ── picker-close! #:token: scoped close ignores a picker it didn't open ────

#[test]
fn picker_close_bang_with_a_stale_token_leaves_a_later_picker_open() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok-a #f)
        (define-command! "go-a" "" (lambda ()
          (set! tok-a (picker! (list (cons "a-item" "pa")) (lambda (x) (log! 'info (to-string "A:" x)))))))
        (define-command! "go-b" "" (lambda ()
          (picker! (list (cons "b-item" "pb")) (lambda (x) (log! 'info (to-string "B:" x))))))
        (define-command! "close-a" "" (lambda () (picker-close! #:token tok-a)))
        "#,
    );
    type_cmd(&mut ed, ":go-a");
    // Replaces A with B — A's on-select already fired (with #f) and drained
    // below, same as `opening_a_second_picker_fires_the_first_callback...`.
    call(&mut ed, "go-b");
    ed.settle();
    ed.state.status_msg = None;

    // A's stale token must not touch B's picker, even though B is what's
    // open right now — the bug this token exists to prevent.
    call(&mut ed, "close-a");
    assert!(
        ed.state.config.picker.is_some(),
        "a stale #:token close must be a no-op, not close whatever picker is open"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "no callback should fire for a stale-token close"
    );

    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "B: pb",
        "B's own on-select must still fire normally"
    );
}

// ── items must be dotted pairs, not proper lists ────────────────────────────

#[test]
fn picker_bang_rejects_proper_list_items_naming_the_arg() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (list "a" "b")) (lambda (x) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "malformed items must not open a picker"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("picker! items"),
        "error should name the offending argument, got {msg:?}"
    );
}

// ── #f payload is reserved for the dismiss signal, not a legal item ────────

#[test]
fn picker_bang_rejects_hash_f_payload() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (cons "a" #f)) (lambda (x) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "a #f payload must not open a picker"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("reserved"),
        "error should explain #f is reserved for dismiss, got {msg:?}"
    );
}

// ── Regression: picker accept mid-frame switching to a shorter buffer ──────

#[test]
fn picker_accept_switching_to_shorter_buffer_mid_frame_does_not_panic() {
    let tmp = safe_tempdir();
    // Buffer A: cursor (head) at char 499 — far beyond buffer B's length.
    let mut ed = editor_from(&format!("{}-[x]>\n", "a".repeat(499)));

    let small = tmp.path().join("small.md");
    std::fs::write(&small, "hi\n").unwrap();
    let path = small.to_string_lossy().replace('\\', "/");

    // Mirrors runtime/plugins/core/pickers/plugin.scm's files-picker on_select:
    // `(switch-to-buffer! (open-buffer! path))` run from a picker callback.
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (picker! (list (cons "small" "{path}"))
                   (lambda (p) (when p (switch-to-buffer! (open-buffer! p)))))))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.config.picker.is_some(), "sanity: picker open");

    let rect = ratatui::layout::Rect::new(0, 0, 40, 12);
    let _ = ed.render_to_buf(rect);

    // Accept: close_picker queues on_select as a PendingWork::Call.
    ed.feed_key(key_enter());

    // The next prepare_frame drains the callback (switching the pane to the
    // 3-char buffer) then renders. Pre-fix, this panicked in ropey's
    // char_to_line: the engine's selection mirror still held buffer A's
    // stale head (499) against buffer B's 3-char rope.
    let _ = ed.render_to_buf(rect);

    let bid = ed.focused_buffer_id();
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "hi\n");

    let pid = ed.state.focused_pane_id;
    let pane = &ed.view.panes[pid];
    assert_eq!(pane.buffer_id, bid);
    assert_eq!(
        pane.selections[pane.primary_idx].head, 0,
        "rendered mirror must reflect buffer B's fresh selection, not A's stale head"
    );
}

// ── Regression: mid-frame switch must scroll the new buffer, same frame ───

#[test]
fn picker_accept_switching_buffers_mid_frame_scrolls_new_buffer_into_view() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");

    // A buffer tall enough that "go to last line" lands well below a 12-row pane.
    let content: String = (0..100).map(|n| format!("line {n}\n")).collect();
    let tall = tmp.path().join("tall.md");
    std::fs::write(&tall, &content).unwrap();
    let path = tall.to_string_lossy().replace('\\', "/");

    // on_select switches buffers, then jumps to the last line — both must
    // land on the NEW buffer within the same prepare_frame the switch runs in.
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (picker! (list (cons "tall" "{path}"))
                   (lambda (p) (when p (begin
                     (switch-to-buffer! (open-buffer! p))
                     (call! "goto-last-line")))))))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.config.picker.is_some(), "sanity: picker open");

    let rect = ratatui::layout::Rect::new(0, 0, 40, 12);
    let _ = ed.render_to_buf(rect);

    // Accept: close_picker queues on_select as a PendingWork::Call.
    ed.feed_key(key_enter());

    // The next prepare_frame drains the callback (switch + goto-last-line on
    // the tall buffer) then must scroll *that* buffer into view before
    // rendering — not the pane's viewport from before the switch.
    let _ = ed.render_to_buf(rect);

    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        content,
        "sanity: switched to the tall buffer"
    );

    let cursor_char = ed.state.panes.state[pid][bid].selections.primary().head();
    let rope = ed.state.buffers.get(bid).text().rope();
    let cursor_line = rope.char_to_line(cursor_char);

    let pane = &ed.view.panes[pid];
    let top = pane.viewport.top_line;
    let bottom = top + pane.viewport.height as usize;
    assert!(
        top > 0,
        "pane must have scrolled down for a last-line cursor in a 100-line buffer, got top_line=0"
    );
    assert!(
        cursor_line >= top && cursor_line < bottom,
        "cursor line {cursor_line} must be visible in viewport [{top}, {bottom}) after a mid-frame switch"
    );
}

// ── Direct host-impl call: the `lsp: None` construction arm ────────────────

#[test]
fn direct_host_impl_open_push_and_close_with_no_lsp_borrow() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PickerOpts, UiHost};

    let mut ed = editor_from("-[a]>bc\n");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let token = host
        .open_picker(
            vec![("one".to_string(), SteelVal::StringV("p1".into()))],
            SteelVal::Void,
            PickerOpts::default(),
        )
        .unwrap();
    assert!(ed.state.config.picker.is_some());

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    assert!(!host.picker_push(token + 1, vec![("x".to_string(), SteelVal::Void)]));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().total_len(), 1);

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.picker_close(None);
    assert!(ed.state.config.picker.is_none());
    assert_eq!(pending_calls(&ed).len(), 1);

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.picker_close(None);
    assert_eq!(
        pending_calls(&ed).len(),
        1,
        "closing with no picker open must be a no-op"
    );
}

#[test]
fn direct_host_impl_picker_close_with_a_stale_token_is_a_no_op() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PickerOpts, UiHost};

    let mut ed = editor_from("-[a]>bc\n");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let token = host
        .open_picker(
            vec![("one".to_string(), SteelVal::StringV("p1".into()))],
            SteelVal::Void,
            PickerOpts::default(),
        )
        .unwrap();

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.picker_close(Some(token + 1));
    assert!(
        ed.state.config.picker.is_some(),
        "a mismatched token must not close the open picker"
    );
    assert!(pending_calls(&ed).is_empty());

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.picker_close(Some(token));
    assert!(
        ed.state.config.picker.is_none(),
        "the matching token must close it"
    );
}

// ── #:query prefill and live requery (#:on-query-change / picker-replace!) ─

#[test]
fn query_prefill_fires_on_query_change_once_queued_not_inline() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void))
               #:query "seed" #:on-query-change (lambda (q) (log! 'info (string-append "q=" q))))))"#,
    );
    ed.state.status_msg = None;
    type_cmd(&mut ed, ":go");
    // The seed fire is scheduled via `(after 0 …)` — a timer-wheel entry, not
    // a `pending_work` `Call` — so it can't be observed via `pending_calls`
    // the way a keystroke's direct `queue_steel_call` can. Its `status_msg`
    // staying unset until `settle()` is what proves it ran deferred, not
    // inline inside `picker!` itself.
    assert!(
        ed.state.status_msg.is_none(),
        "must not have run yet — still queued"
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "q=seed",
        "the queued fire must have run with the prefilled query"
    );
    let guard = ed.state.picker_view.read().unwrap();
    assert_eq!(guard.as_ref().expect("picker open").query, "seed");
}

#[test]
fn empty_query_prefill_fires_nothing() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void))
               #:on-query-change (lambda (q) (log! 'info (string-append "q=" q))))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert!(
        pending_calls(&ed).is_empty(),
        "an empty (default) #:query must not fire #:on-query-change on open"
    );
}

#[test]
fn on_query_change_fires_once_per_keystroke_queued_not_inline() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void))
               #:on-query-change (lambda (q) (log! 'info (string-append "q=" q))))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.state.status_msg = None;

    ed.feed_key(key('a'));
    assert_eq!(
        pending_calls(&ed).len(),
        1,
        "a query-changing keystroke must queue the callback, not invoke it inline"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "must not have run yet — still queued"
    );
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "q=a");

    ed.state.status_msg = None;
    ed.feed_key(key_backspace());
    assert_eq!(pending_calls(&ed).len(), 1);
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "q=",
        "backspacing to empty is still a real query change"
    );

    // Backspace on an already-empty query is a documented no-op — must not
    // fire a second time for nothing.
    ed.state.status_msg = None;
    ed.feed_key(key_backspace());
    assert!(pending_calls(&ed).is_empty());
    ed.settle();
    assert!(ed.state.status_msg.is_none());
}

#[test]
fn non_live_picker_never_fires_on_query_change() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1")) (lambda (x) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    ed.feed_key(key('z'));
    assert!(
        pending_calls(&ed).is_empty(),
        "a picker opened without #:on-query-change must never queue a query-change callback"
    );
}

// ── #:debounce-ms ───────────────────────────────────────────────────────────

#[test]
fn debounced_seed_fire_is_not_delayed_by_the_window() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void))
               #:query "seed" #:debounce-ms 100000
               #:on-query-change (lambda (q) (log! 'info (string-append "q=" q))))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.settle();

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "q=seed",
        "the #:query seed must fire immediately, not wait out a 100000ms debounce window"
    );
}

#[test]
fn debounced_query_change_fires_once_after_the_window_not_per_keystroke() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void))
               #:debounce-ms 0
               #:on-query-change (lambda (q) (log! 'warn (string-append "fired:" q))))))"#,
    );
    type_cmd(&mut ed, ":go");

    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key('c'));
    // First settle() drains the three queued wrapped-callback calls, each
    // cancelling the last and re-arming the 0ms debounce timer — leaving one
    // freshly-armed timer that this same settle() pass, having already run
    // its own drain_async_sources before draining pending_work, doesn't yet
    // see as due. A second settle() catches it.
    ed.settle();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert_eq!(
        log.matches("fired:").count(),
        1,
        "three rapid keystrokes must collapse to one trailing fire, not one per keystroke: {log:?}"
    );
    assert!(
        log.contains("fired:abc"),
        "the one fire must carry the latest query, not an earlier keystroke's: {log:?}"
    );
}

#[test]
fn debounce_stops_and_clears_the_previous_source_immediately_every_keystroke() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1")) (lambda (x) (void))
               #:debounce-ms 100000
               #:on-query-change (lambda (q) (log! 'info (string-append "q=" q))))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.state.config.picker.as_ref().unwrap().total_len(), 1);

    ed.feed_key(key('a'));
    ed.settle();

    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        0,
        "stop-and-clear must run on every keystroke immediately, not wait for the \
         100000ms debounce window to elapse"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "the debounced callback itself must not have fired yet"
    );
}

#[test]
fn debounced_backspace_to_empty_cancels_a_still_pending_nonempty_query_fire() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void))
               #:debounce-ms 0
               #:on-query-change (lambda (q)
                 (unless (equal? q "") (log! 'warn (string-append "spawned:" q)))))))"#,
    );
    type_cmd(&mut ed, ":go");

    ed.feed_key(key('a'));
    ed.feed_key(key_backspace());
    // Two settles, as above: the first drains the two queued wrapped-
    // callback calls (arming, then re-arming, the 0ms timer); the second
    // lets the surviving timer actually fire — without it, this test would
    // pass even if backspace failed to cancel the "a" timer, since neither
    // would have fired yet either way.
    ed.settle();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("spawned:a"),
        "backspacing to empty before the window elapses must cancel the earlier \
         non-empty keystroke's pending fire, not let it spawn a stale search: {log:?}"
    );
    assert!(
        log.is_empty(),
        "the surviving empty-query fire must itself be a no-op: {log:?}"
    );
}

#[test]
fn debounce_ms_without_on_query_change_errors() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void)) #:debounce-ms 150)))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "#:debounce-ms without #:on-query-change must error before opening a picker"
    );
}

#[test]
fn debounce_ms_rejects_a_non_callable_on_query_change() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void))
               #:debounce-ms 150 #:on-query-change '())))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "a non-callable #:on-query-change must be rejected at open time even under #:debounce-ms"
    );
}

#[test]
fn debounce_ms_rejects_a_negative_value() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list) (lambda (x) (void))
               #:debounce-ms -1 #:on-query-change (lambda (q) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "a negative #:debounce-ms must be rejected"
    );
}

#[test]
fn picker_replace_bang_swaps_items_instead_of_appending() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! (list (cons "one" "p1")) (lambda (x) (log! 'info (to-string x)))))))
        (define-command! "replace-real" "" (lambda ()
          (log! 'info (to-string (picker-replace! tok (list (cons "two" "p2")))))))
        (define-command! "replace-stale" "" (lambda ()
          (log! 'info (to-string (picker-replace! (+ tok 1) (list (cons "three" "p3")))))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.state.config.picker.as_ref().unwrap().total_len(), 1);

    call(&mut ed, "replace-real");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#true");
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        1,
        "replace must swap the item list, not append to it"
    );

    call(&mut ed, "replace-stale");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        1,
        "a stale-token replace must not apply"
    );
}

#[test]
fn picker_source_stop_bang_matches_the_real_token_and_rejects_a_stale_one() {
    // No source needs to be attached: the token guard is the whole contract
    // under test, and `picker-source-stop!` returns whether `token` matched
    // the open session regardless of whether a source was actually
    // attached (see `UiHost::picker_source_stop`'s doc).
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "stop-stale" "" (lambda ()
          (log! 'info (to-string (picker-source-stop! (+ tok 1))))))
        (define-command! "stop-real" "" (lambda ()
          (log! 'info (to-string (picker-source-stop! tok)))))
        "#,
    );
    type_cmd(&mut ed, ":go");

    call(&mut ed, "stop-stale");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(
        ed.state.config.picker.is_some(),
        "a stale-token stop must not touch the open picker"
    );

    call(&mut ed, "stop-real");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#true");
    assert!(
        ed.state.config.picker.is_some(),
        "picker-source-stop! must not close the picker, only detach its source"
    );
}
