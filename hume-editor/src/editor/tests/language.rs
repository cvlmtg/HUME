use super::*;

use crate::testing::MockHost;
use hume_scripting::ScriptingHost;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Attach a scripting host to `ed`, optionally evaluating `src` in init mode.
pub(super) fn attach_host(ed: &mut Editor, src: &str) {
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    if !src.is_empty() {
        host.eval_source(src, &mut mock).expect("eval failed");
    }
    ed.scripting = Some(host);
}

/// Register rust-only identities into `ed.state.config.languages` directly (no Scheme eval).
fn register_rust(ed: &mut Editor, name: &str, exts: &[&str]) {
    ed.state
        .config
        .languages
        .register_identity_no_rebuild(name, exts, &[], &[], None);
    ed.state
        .config
        .languages
        .rebuild_glob_set()
        .expect("rebuild ok");
}

// ── Buffer.language round-trip ────────────────────────────────────────────────

#[test]
fn set_buffer_language_writes_language_field() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust")
    );
    // A different language must not match.
    assert_ne!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("python")
    );
}

#[test]
fn set_buffer_language_to_none_clears_language() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.set_buffer_language(bid, None);
    assert!(ed.state.buffers.get(bid).language.is_none());
}

#[test]
fn set_buffer_language_no_op_when_unchanged() {
    let mut ed = editor_from("-[a]>b\n");
    // No scripting host: if set_buffer_language fires the hook anyway, it would
    // panic because scripting is None. This test verifies no-op short-circuit.
    let bid = ed.focused_buffer_id();
    // Start with no language: setting to None must not panic.
    ed.set_buffer_language(bid, None);
    assert!(ed.state.buffers.get(bid).language.is_none());
    // Now set a language and repeat: second set must short-circuit without panic.
    ed.scripting = Some(ScriptingHost::new());
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)); // no-op, no double-fire
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust")
    );
}

// ── detect_and_set_language ───────────────────────────────────────────────────

#[test]
fn detect_and_set_language_matches_extension() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    // Give the buffer a .rs path so detection can match it.
    ed.state.buffers.get_mut(bid).path = Some(std::path::PathBuf::from("/tmp/foo.rs"));
    register_rust(&mut ed, "rust", &["rs"]);
    ed.detect_and_set_language(bid);
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust")
    );
    // Detection of a registered ext must leave a language set.
    assert!(ed.state.buffers.get(bid).language.is_some());
}

#[test]
fn detect_and_set_language_no_match_leaves_none() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    // Buffer has no path, so no detection possible.
    assert!(ed.state.buffers.get(bid).path().is_none());
    ed.detect_and_set_language(bid);
    assert!(ed.state.buffers.get(bid).language.is_none());
}

/// `open-buffer!` then setting its `language` option on the same new buffer, in one
/// eval. `apply_script_effects`'s tail (`detect_pending_languages`) must not
/// re-detect the freshly-opened buffer over the explicit assertion the same
/// eval *just* made (the `SetBufferLanguage` effect applies first, earlier
/// in the same effect log). Detection would pick "rust" from the
/// `.rs` extension; the explicit `set-buffer-option!` call asks for
/// "notes"; the explicit call must win.
#[test]
fn open_buffer_then_set_buffer_language_in_one_eval_keeps_the_explicit_value() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    register_rust(&mut ed, "rust", &["rs"]);
    ed.state
        .config
        .languages
        .register_identity_no_rebuild("notes", &[], &[], &[], None);
    ed.state
        .config
        .languages
        .rebuild_glob_set()
        .expect("rebuild ok");

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let file_str = steel_path(&file);

    let mut host = hume_scripting::ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-typed-command! "go" "" (lambda ()
                 (define b (open-buffer! {file_str}))
                 (set-buffer-option! b "language" "notes")))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":go");

    let bid = ed
        .state
        .buffers
        .find_by_path(&file.canonicalize().unwrap())
        .expect("the opened buffer must be findable by path");
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .language
            .map(|id| ed.state.config.languages.name_of(id)),
        Some("notes"),
        "the explicit language set-buffer-option! call must win over what plain \
         detection would have found from the .rs extension"
    );
    assert!(
        ed.state.buffers.get(bid).language_explicit,
        "the buffer must be marked explicit, not left looking auto-detected"
    );
}

/// `(set-buffer-option! pane "language" …)` parses its value the way
/// `:set buffer language=` does: `""` clears the language, an unregistered
/// name is still applied but reported, and `get-buffer-option` reads a
/// change queued earlier in the same eval.
#[test]
fn language_option_parses_like_set_and_reads_back_a_queued_change() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    register_rust(&mut ed, "rust", &["rs"]);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "to-rust" "" (lambda (bid)
             (set-buffer-option! bid "language" "rust")
             (log! 'info (get-buffer-option bid "language"))))
           (define-typed-command! "to-none" "" (lambda (bid)
             (set-buffer-option! bid "language" "")
             (log! 'info (string-append "[" (get-buffer-option bid "language") "]"))))
           (define-typed-command! "to-unknown" "" (lambda (bid)
             (set-buffer-option! bid "language" "no-such-lang")))"#,
    );
    let bid = ed.focused_buffer_id();

    type_cmd(&mut ed, ":to-rust");
    assert_eq!(ed.state.status_msg.as_deref(), Some("rust"));
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust")
    );

    type_cmd(&mut ed, ":to-none");
    assert_eq!(ed.state.status_msg.as_deref(), Some("[]"));
    assert_eq!(ed.state.buffers.get(bid).language, None, "\"\" clears");

    type_cmd(&mut ed, ":to-unknown");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("language 'no-such-lang' is not registered"),
        "an unregistered name must be reported"
    );
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("no-such-lang"),
        "an unregistered name is still applied"
    );
}

// ── :set buffer language= intercept ──────────────────────────────────────────

#[test]
fn typed_set_language_global_scope_errors() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let result = run_set(&mut ed, "global language=rust");
    assert!(result.is_err(), "global language must be an error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("per-buffer"),
        "error should mention per-buffer: {msg}"
    );
}

#[test]
fn typed_set_language_buffer_scope_sets_language() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    register_rust(&mut ed, "rust", &["rs"]);
    let bid = ed.focused_buffer_id();
    run_set(&mut ed, "buffer language=rust").expect(":set buffer language=rust failed");
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust")
    );
}

#[test]
fn typed_set_language_empty_value_clears_language() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    run_set(&mut ed, "buffer language=").expect(":set buffer language= failed");
    assert!(ed.state.buffers.get(bid).language.is_none());
}

#[test]
fn typed_set_language_unknown_warns_but_sets() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    // "unknown-lang" is not registered: should warn but still set.
    let result = run_set(&mut ed, "buffer language=unknown-lang");
    assert!(
        result.is_ok(),
        "unknown language must not error, got: {result:?}"
    );
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("unknown-lang")
    );
}

// ── OnLanguageSet hook fires ──────────────────────────────────────────────────

#[test]
fn on_language_set_hook_fires_on_set_buffer_language() {
    let mut ed = editor_from("-[a]>b\n");
    // Register hook that moves right when on-language-set fires.
    attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))"#,
    );
    let bid = ed.focused_buffer_id();
    let before = state(&ed);
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();
    // move-right from hook must have moved the cursor.
    assert_ne!(state(&ed), before, "on-language-set hook must have fired");
}

#[test]
fn on_language_set_hook_does_not_fire_on_no_op() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))"#,
    );
    let bid = ed.focused_buffer_id();
    // Set once to establish baseline.
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    let after_first = state(&ed);
    // Set same value again: should be a no-op; hook must not fire.
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    assert_eq!(
        state(&ed),
        after_first,
        "hook must not fire on unchanged language"
    );
}

/// `(set-buffer-option! bid "language" "rust") (close-buffer!
/// bid)` in one eval must not panic. The `Effect::SetBufferLanguage` this
/// queues only applies after the eval returns (`apply_script_effects`
/// drains the effect vec after the whole body ran), so by the time it
/// applies, `bid` (closed by the same body) is already gone; the effect
/// arm must check liveness itself rather than let `set_buffer_language_
/// explicit`'s panicking `get_mut` hit an unseeded slot.
///
/// Without the `try_get` guard in `apply_script_effects`'s
/// `SetBufferLanguage` arm, this panics with `BufferStore: unseeded BufferId`.
#[test]
fn set_buffer_language_then_close_in_one_eval_does_not_panic() {
    use hume_editing::text::BufferText;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    // A second buffer so closing `bid` frees its slot outright rather than
    // hitting the unrelated last-buffer scratch-replacement branch.
    ed.open_buffer(Buffer::at_start(BufferText::from("x\n")));
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "set-then-close" "" (lambda (bid)
             (set-buffer-option! bid "language" "rust")
             (close-buffer! bid)))"#,
    );
    let bid = ed.focused_buffer_id();

    type_cmd(&mut ed, ":set-then-close");

    assert!(
        ed.state.buffers.try_get(bid).is_none(),
        "bid must actually be closed"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("no longer exists"),
        "the dead bid must be reported, not silently dropped: {log:?}"
    );
}
