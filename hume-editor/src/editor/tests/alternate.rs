use super::*;
use hume_scripting::host::CommandHost;
use pretty_assertions::assert_eq;

// ── Helpers ───────────────────────────────────────────────────────────────────

// ── alternate_buffer() ────────────────────────────────────────────────────────

#[test]
fn alternate_buffer_none_with_single_buffer() {
    let ed = editor_from("-[h]>ello\n");
    assert_eq!(ed.alternate_buffer(), None);
}

// ── goto-alternate-buffer  ────────────────────────────────────────────────────

#[test]
fn goto_alternate_buffer_warns_when_no_alternate() {
    let mut ed = editor_from("-[h]>ello\n");
    let id_before = ed.focused_buffer_id();
    live_host!(ed)
        .run_command_sync("goto-alternate-buffer", Some(1), false, None)
        .expect("goto-alternate-buffer must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_before,
        "no buffer change with no alternate"
    );
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("warning should be reported");
    assert!(
        msg.contains("No alternate buffer"),
        "unexpected status: {msg:?}"
    );
}

// ── %/# expansion in typed commands ──────────────────────────────────────────

#[test]
fn colon_e_hash_errors_with_no_alternate() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":e #");
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("error should be reported");
    assert!(
        msg.contains("No alternate buffer"),
        "unexpected status: {msg:?}"
    );
}

#[test]
fn colon_e_percent_errors_with_no_path() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":e %");
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("error should be reported");
    assert!(msg.contains("No file name"), "unexpected status: {msg:?}");
}

// ── goto-alternate-buffer in registry ──────────────────────────────────────────

#[test]
fn goto_alternate_buffer_is_registered_as_jump() {
    let reg = super::super::registry::CommandRegistry::with_defaults();
    let cmd = reg
        .get_mappable("goto-alternate-buffer")
        .expect("goto-alternate-buffer must be registered");
    assert!(
        cmd.meta().is_jump,
        "goto-alternate-buffer must have jump:true"
    );
}
