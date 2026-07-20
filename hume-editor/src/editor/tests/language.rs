use super::*;

use crate::editor::error::CommandError;
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

/// Register rust-only identities into `ed.state.languages` directly (no Scheme eval).
fn register_rust(ed: &mut Editor, name: &str, exts: &[&str]) {
    ed.state
        .languages
        .register_identity_no_rebuild(name, exts, &[], &[]);
    ed.state.languages.rebuild_glob_set().expect("rebuild ok");
}

// ── Buffer.language round-trip ────────────────────────────────────────────────

#[test]
fn set_buffer_language_writes_language_field() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.languages.id_of("rust")
    );
    // Flip: wrong language must not match.
    assert_ne!(
        ed.state.buffers.get(bid).language,
        ed.state.languages.id_of("python")
    );
}

#[test]
fn set_buffer_language_to_none_clears_language() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.languages.intern("rust");
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
    // Start with no language — setting to None must not panic.
    ed.set_buffer_language(bid, None);
    assert!(ed.state.buffers.get(bid).language.is_none());
    // Now set a language and repeat — second set must short-circuit without panic.
    ed.scripting = Some(ScriptingHost::new());
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)); // no-op, no double-fire
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.languages.id_of("rust")
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
        ed.state.languages.id_of("rust")
    );
    // Flip: the language must not be absent after detection of a registered ext.
    assert!(ed.state.buffers.get(bid).language.is_some());
}

#[test]
fn detect_and_set_language_no_match_leaves_none() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    // Buffer has no path — no detection possible.
    assert!(ed.state.buffers.get(bid).path().is_none());
    ed.detect_and_set_language(bid);
    assert!(ed.state.buffers.get(bid).language.is_none());
}

// ── :set buffer language= intercept ──────────────────────────────────────────

fn run_cmd(ed: &mut Editor, cmd: &str) -> Result<(), CommandError> {
    crate::editor::commands::typed_set(ed, Some(cmd), false)
}

#[test]
fn typed_set_language_global_scope_errors() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let result = run_cmd(&mut ed, "global language=rust");
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
    run_cmd(&mut ed, "buffer language=rust").expect(":set buffer language=rust failed");
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.languages.id_of("rust")
    );
}

#[test]
fn typed_set_language_empty_value_clears_language() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    run_cmd(&mut ed, "buffer language=").expect(":set buffer language= failed");
    assert!(ed.state.buffers.get(bid).language.is_none());
}

#[test]
fn typed_set_language_unknown_warns_but_sets() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let bid = ed.focused_buffer_id();
    // "unknown-lang" is not registered — should warn but still set.
    let result = run_cmd(&mut ed, "buffer language=unknown-lang");
    assert!(
        result.is_ok(),
        "unknown language must not error, got: {result:?}"
    );
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.languages.id_of("unknown-lang")
    );
}

// ── OnLanguageSet hook fires ──────────────────────────────────────────────────

#[test]
fn on_language_set_hook_fires_on_set_buffer_language() {
    let mut ed = editor_from("-[a]>b\n");
    // Register hook that moves right when on-language-set fires.
    attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let bid = ed.focused_buffer_id();
    let before = state(&ed);
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.drain_hooks();
    // move-right from hook must have moved the cursor.
    assert_ne!(state(&ed), before, "on-language-set hook must have fired");
}

#[test]
fn on_language_set_hook_does_not_fire_on_no_op() {
    let mut ed = editor_from("-[a]>b\n");
    attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let bid = ed.focused_buffer_id();
    // Set once to establish baseline.
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    let after_first = state(&ed);
    // Set same value again — should be a no-op; hook must not fire.
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    assert_eq!(
        state(&ed),
        after_first,
        "hook must not fire on unchanged language"
    );
}
