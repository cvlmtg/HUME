// The Steel surface for the fuzzy picker (docs/FUZZY-FINDERS.md B4):
// (picker! items on-select #:prompt "…") / (picker-push! token items) /
// (picker-close!). The Rust store/widget/key-handling underneath is
// covered by `tests/picker.rs`, which builds sessions directly — this
// file exercises the builtins themselves end to end through real Steel
// source, mirroring `lsp_drawer.rs`'s `run`/`arm_*` pattern.
//
// The picker is full-modal (Q-B7, `docs/FUZZY-FINDERS.md`): once one is
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
    let tmp = tempfile::tempdir().unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
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
    ed.prepare_frame(40, 12, &mut ctx);

    ed.feed_key(key('t'));
    ed.feed_key(key('w'));
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "p2");
    assert!(ed.state.picker.is_none());

    // LESSONS.md L4: don't stop at the terminal action — keep interacting
    // and confirm ordinary editing resumes with no further callback fire.
    ed.state.status_msg = None;
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();

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
    let tmp = tempfile::tempdir().unwrap();
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
    assert_eq!(ed.state.picker.as_ref().unwrap().total_len(), 1);

    call(&mut ed, "push-real");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#true");
    assert_eq!(ed.state.picker.as_ref().unwrap().total_len(), 2);

    call(&mut ed, "push-stale");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert_eq!(
        ed.state.picker.as_ref().unwrap().total_len(),
        2,
        "a stale-token push must not apply"
    );

    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();
    assert!(ed.state.picker.is_none());

    call(&mut ed, "push-real");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "#false",
        "pushing after the picker closed must be a silent #f, not an error"
    );
    assert!(ed.state.picker.is_none());
}

// ── picker! over an already-open picker fires the old callback once ────────

#[test]
fn opening_a_second_picker_fires_the_first_callback_with_false_exactly_once() {
    let tmp = tempfile::tempdir().unwrap();
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
        ed.state.pending_steel_calls.len(),
        1,
        "replacing the open picker must queue exactly one callback (the old one, with #f)"
    );
    ed.drain_pending_steel_calls();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "A: #false");

    ed.state.status_msg = None;
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "B: pb");
    assert!(ed.state.picker.is_none());
}

// ── picker-close!: fires #f exactly once, idempotent, keeps L4 discipline ──

#[test]
fn picker_close_bang_fires_false_once_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (cons "one" "p1")) (lambda (x) (log! 'info (to-string x))))))
           (define-command! "close-it" "" (lambda () (picker-close!)))"#,
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.picker.is_some());

    call(&mut ed, "close-it");
    assert!(ed.state.picker.is_none());
    assert_eq!(ed.state.pending_steel_calls.len(), 1);

    call(&mut ed, "close-it");
    assert_eq!(
        ed.state.pending_steel_calls.len(),
        1,
        "closing an already-closed picker must not queue a second callback"
    );

    ed.drain_pending_steel_calls();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");

    // LESSONS.md L4: keep interacting past the terminal action.
    ed.state.status_msg = None;
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();

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

// ── Q-B3: items must be dotted pairs, not proper lists ─────────────────────

#[test]
fn picker_bang_rejects_proper_list_items_naming_the_arg() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (list "a" "b")) (lambda (x) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.picker.is_none(),
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
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (picker! (list (cons "a" #f)) (lambda (x) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.picker.is_none(),
        "a #f payload must not open a picker"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("reserved"),
        "error should explain #f is reserved for dismiss, got {msg:?}"
    );
}

// ── Direct host-impl call: the `lsp: None` construction arm ────────────────

#[test]
fn direct_host_impl_open_push_and_close_with_no_lsp_borrow() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[a]>bc\n");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let token = host
        .open_picker(
            vec![("one".to_string(), SteelVal::StringV("p1".into()))],
            String::new(),
            SteelVal::Void,
        )
        .unwrap();
    assert!(ed.state.picker.is_some());

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    assert!(!host.picker_push(token + 1, vec![("x".to_string(), SteelVal::Void)]));
    assert_eq!(ed.state.picker.as_ref().unwrap().total_len(), 1);

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.picker_close();
    assert!(ed.state.picker.is_none());
    assert_eq!(ed.state.pending_steel_calls.len(), 1);

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.picker_close();
    assert_eq!(
        ed.state.pending_steel_calls.len(),
        1,
        "closing with no picker open must be a no-op"
    );
}
