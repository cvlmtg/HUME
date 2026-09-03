// The Steel surface for the fuzzy picker:
// (picker! items on-select #:prompt "…") / (live-picker! on-select #:command …) /
// (picker-push! token items) / (picker-replace! token items) / (picker-close!).
// The Rust store/widget/key-handling underneath is covered by
// `tests/picker.rs`, which builds sessions directly — this file exercises
// the builtins themselves end to end through real Steel source, mirroring
// `lsp_drawer.rs`'s `run`/`arm_*` pattern.
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

use hume_grid::Rect;

use super::*;
use hume_engine::pipeline::RenderContext;
use hume_scripting::host::TruncateEnd;
use steel::rvals::SteelVal;

fn call(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false);
}

fn editor_with(source: &str) -> (Editor, tempfile::TempDir) {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(&mut ed, tmp.path(), source);
    (ed, tmp)
}

// ── picker! opens a session and returns its token ─────────────────────────

#[test]
fn picker_bang_opens_session_and_returns_its_token() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
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

// ── #:truncate ──────────────────────────────────────────────────────────

#[test]
fn picker_bang_truncate_defaults_to_head() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1")) (lambda (x) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    let session = ed
        .state
        .config
        .picker
        .as_ref()
        .expect("picker! must open a session");
    assert_eq!(session.truncate(), TruncateEnd::Head);
}

#[test]
fn picker_bang_truncate_tail_reaches_the_session() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1")) (lambda (x) (void)) #:truncate 'tail)))"#,
    );
    type_cmd(&mut ed, ":go");

    let session = ed
        .state
        .config
        .picker
        .as_ref()
        .expect("picker! must open a session");
    assert_eq!(session.truncate(), TruncateEnd::Tail);
}

#[test]
fn picker_bang_truncate_rejects_an_unknown_symbol() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1")) (lambda (x) (void)) #:truncate 'middle)))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "an unrecognized #:truncate value must not open a picker"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("'head") && msg.contains("'tail"),
        "error should name both accepted values, got {msg:?}"
    );
}

#[test]
fn live_picker_bang_truncate_tail_reaches_the_session() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void))
               #:command (lambda (q) #f) #:truncate 'tail)))"#,
    );
    type_cmd(&mut ed, ":go");

    let session = ed
        .state
        .config
        .picker
        .as_ref()
        .expect("live-picker! must open a session");
    assert_eq!(session.truncate(), TruncateEnd::Tail);
}

// ── End-to-end: open, type a query, accept, keep interacting (LESSONS L4) ──

#[test]
fn end_to_end_accept_fires_payload_then_normal_editing_resumes() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
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
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-typed-command! "go" "" (lambda ()
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
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go-a" "" (lambda ()
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
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
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
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok-a #f)
        (define-typed-command! "go-a" "" (lambda ()
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
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
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
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
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
            r#"(define-typed-command! "go" "" (lambda ()
                 (picker! (list (cons "small" "{path}"))
                   (lambda (p) (when p (switch-to-buffer! (open-buffer! p)))))))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.config.picker.is_some(), "sanity: picker open");

    let rect = Rect::new(0, 0, 40, 12);
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
            r#"(define-typed-command! "go" "" (lambda ()
                 (picker! (list (cons "tall" "{path}"))
                   (lambda (p) (when p (begin
                     (switch-to-buffer! (open-buffer! p))
                     (call! "goto-last-line")))))))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.config.picker.is_some(), "sanity: picker open");

    let rect = Rect::new(0, 0, 40, 12);
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
    use hume_scripting::host::{PickerFeedMode, PickerOpts, UiHost};

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
    assert!(!host.picker_feed(
        token + 1,
        vec![("x".to_string(), SteelVal::Void)],
        PickerFeedMode::Append
    ));
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

// ── picker! does not wire up live-requery keywords ─────────────────────────

#[test]
fn picker_bang_silently_ignores_an_on_query_change_keyword() {
    // Steel's keyword-arg calling convention doesn't reject a keyword the
    // callee's signature never declares (extra `#:key val` pairs are simply
    // unused) — so passing #:on-query-change to picker! doesn't raise. What
    // must hold is that it's dead: `picker!`'s signature has no
    // `#:on-query-change` parameter, and never wires the value to anything —
    // live requery lives on `live-picker!` instead.
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1")) (lambda (x) (void))
               #:on-query-change (lambda (q) (log! 'info "must never fire")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        1,
        "picker! must still open normally with an unrecognized extra keyword"
    );

    ed.feed_key(key('z'));
    assert!(
        pending_calls(&ed).is_empty(),
        "an extra #:on-query-change argument on picker! must never be wired to anything \
         — live requery moved to live-picker!"
    );
}

// ── live-picker!: #:query seed spawns synchronously ─────────────────────────

#[test]
fn live_picker_seed_spawn_is_synchronous_and_not_debounced() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void))
               #:query "seed" #:debounce-ms 100000
               #:command (lambda (q) (log! 'info (string-append "q=" q)) #f))))"#,
    );
    ed.state.status_msg = None;
    type_cmd(&mut ed, ":go");

    // No settle() at all — the seed spawn runs synchronously inside
    // live-picker! itself, before the command that called it even returns.
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "q=seed",
        "the #:query seed must fire #:command synchronously, not wait out a \
         100000ms debounce window or a settle() drain"
    );
}

#[test]
fn live_picker_empty_seed_calls_no_builder() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void))
               #:command (lambda (q) (log! 'info (string-append "q=" q)) #f))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert!(
        ed.state.status_msg.is_none(),
        "an empty (default) #:query must not fire #:command on open"
    );
}

// ── live-picker!: keystrokes stop-and-clear immediately, respawn debounced ──

#[test]
fn live_picker_keystroke_keeps_previous_rows_until_the_new_search_delivers() {
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-typed-command! "go" "" (lambda ()
          (set! tok (live-picker! (lambda (x) (void))
            #:debounce-ms 100000
            #:command (lambda (q) (log! 'info (string-append "q=" q)) #f)))))
        (define-command! "seed-row" "" (lambda ()
          (picker-push! tok (list (cons "one" "p1")))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "seed-row");
    assert_eq!(ed.state.config.picker.as_ref().unwrap().total_len(), 1);

    ed.feed_key(key('a'));
    ed.settle();

    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        1,
        "the previous pattern's rows must stay on screen through the whole \
         100000ms debounce window, not clear immediately on the keystroke"
    );
    assert!(
        ed.state.config.picker.as_ref().unwrap().is_pending(),
        "a live query change must mark the session pending even while the \
         stale rows are still the only ones on screen"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "the debounced #:command builder itself must not have fired yet"
    );
}

#[test]
fn live_picker_debounced_respawn_raise_clears_stale_rows_and_unsticks_pending() {
    // `#:command` raising after the debounce (a bad user builder, or
    // `picker-source-spawn!` itself failing to spawn) must not stick the
    // session in the "requery in flight" state forever — the previous
    // pattern's now-orphaned rows must clear and `is_pending` must settle
    // back to false, the same outcome a successful requery with nothing to
    // show reaches.
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-typed-command! "go" "" (lambda ()
          (set! tok (live-picker! (lambda (x) (void))
            #:debounce-ms 0
            #:command (lambda (q) (error "boom"))))))
        (define-command! "seed-row" "" (lambda ()
          (picker-push! tok (list (cons "stale" "p")))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "seed-row");
    assert_eq!(ed.state.config.picker.as_ref().unwrap().total_len(), 1);

    ed.feed_key(key('a'));
    // Two settles, same as every other debounce-ms-0 test here: the first
    // drains the query-change callback (stop, arm the 0ms timer), the
    // second lets that timer fire and run `#:command`, which raises.
    ed.settle();
    ed.settle();

    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        0,
        "a raise on the debounced respawn must still drop the previous \
         pattern's stale rows"
    );
    assert!(
        !ed.state.config.picker.as_ref().unwrap().is_pending(),
        "a raise on the debounced respawn must not leave the session \
         permanently marked as a requery in flight"
    );
}

#[test]
fn live_picker_rapid_keystrokes_collapse_to_one_trailing_builder_call_with_the_latest_query() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void))
               #:debounce-ms 0
               #:command (lambda (q) (log! 'warn (string-append "fired:" q)) #f))))"#,
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
fn live_picker_backspace_to_empty_cancels_a_pending_nonempty_spawn() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void))
               #:debounce-ms 0
               #:command (lambda (q)
                 (unless (equal? q "") (log! 'warn (string-append "spawned:" q)))
                 #f))))"#,
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

// ── live-picker!: never locally fuzzy-filters ────────────────────────────────

#[test]
fn live_picker_rows_keep_source_order_regardless_of_query() {
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-typed-command! "go" "" (lambda ()
          (set! tok (live-picker! (lambda (x) (void))
            #:query "zzz-does-not-fuzzy-match-anything"
            #:debounce-ms 100000
            #:command (lambda (q) #f)))))
        (define-command! "seed-rows" "" (lambda ()
          (picker-push! tok (list (cons "b" "b") (cons "a" "a") (cons "c" "c")))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "seed-rows");

    let picker = ed.state.config.picker.as_ref().unwrap();
    assert_eq!(
        picker.window(10).collect::<Vec<_>>(),
        vec!["b", "a", "c"],
        "a live session must never locally fuzzy-filter, even against a query that \
         would fuzzy-match nothing"
    );
}

// ── live-picker!: token scopes picker-close! same as picker! ────────────────

#[test]
fn live_picker_token_scopes_picker_close() {
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-typed-command! "go" "" (lambda ()
          (set! tok (live-picker! (lambda (x) (log! 'info (to-string x)))
            #:command (lambda (q) #f)))))
        (define-command! "close-stale" "" (lambda ()
          (picker-close! #:token (+ tok 1))))
        (define-command! "close-real" "" (lambda ()
          (picker-close! #:token tok)))
        "#,
    );
    type_cmd(&mut ed, ":go");

    call(&mut ed, "close-stale");
    assert!(
        ed.state.config.picker.is_some(),
        "a stale-token close must leave the live picker open"
    );

    call(&mut ed, "close-real");
    assert!(ed.state.config.picker.is_none());
}

// ── live-picker! validation ───────────────────────────────────────────────

#[test]
fn live_picker_rejects_a_non_callable_command() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void)) #:command '())))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "a non-callable #:command must be rejected before opening a picker"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("command"),
        "error should name the offending argument, got {msg:?}"
    );
}

#[test]
fn live_picker_rejects_a_negative_debounce_ms() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void))
               #:debounce-ms -1 #:command (lambda (q) #f))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "a negative #:debounce-ms must be rejected"
    );
}

#[test]
fn live_picker_requires_command() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.picker.is_none(),
        "live-picker! without #:command must error before opening a picker"
    );
}

#[test]
fn live_picker_rejects_a_builder_return_that_is_not_an_argv_list() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-typed-command! "go" "" (lambda ()
             (live-picker! (lambda (x) (void))
               #:query "seed"
               #:command (lambda (q) 42))))"#,
    );
    type_cmd(&mut ed, ":go");

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("#:command"),
        "a builder return that isn't #f or an argv list must raise naming #:command, got {msg:?}"
    );
    // The raise happens after `%live-picker!` has already installed the
    // session — `live-picker!` never returns a token to bind, but the
    // picker itself is left open (Esc still closes it), not torn down.
    assert!(
        ed.state.config.picker.is_some(),
        "a seed-spawn raise must leave the already-opened picker in place"
    );
}

#[test]
fn picker_replace_bang_swaps_items_instead_of_appending() {
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-typed-command! "go" "" (lambda ()
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
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-typed-command! "go" "" (lambda ()
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
