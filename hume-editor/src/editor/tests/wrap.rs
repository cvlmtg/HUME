use super::*;

use crate::editor::error::CommandError;
use hume_engine::pane::WrapMode;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn run_set(ed: &mut Editor, cmd: &str) -> Result<(), CommandError> {
    crate::editor::commands::typed_set(ed, Some(cmd), false)
}

fn focused_pane(ed: &Editor) -> &hume_engine::pane::Pane {
    &ed.view.panes[ed.state.focused_pane_id]
}

// ── Generic scope/key validation (`setting_scopes`) ─────────────────────────
//
// These lock in the dedup from the `typed_set` refactor: one data-driven
// check now produces every "unknown setting" / "wrong scope for this
// setting" message, instead of two divergent hardcoded strings.

#[test]
fn typed_set_unknown_setting_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "global foo=bar");
    assert!(result.is_err(), "nonexistent setting must error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("unknown setting") && msg.contains("foo"),
        "error should say the setting is unknown: {msg}"
    );
}

#[test]
fn typed_set_garbage_scope_on_real_key_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "bogus tab-width=2");
    assert!(result.is_err(), "garbage scope token must error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("tab-width") && msg.contains("global") && msg.contains("buffer"),
        "error should name the key and its real valid scopes: {msg}"
    );
}

// ── `:set pane wrap-mode=…` ─────────────────────────────────────────────────

#[test]
fn typed_set_pane_wrap_mode_updates_pane_and_saved() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=word").expect(":set pane wrap-mode=word failed");
    let pane = focused_pane(&ed);
    assert_eq!(pane.wrap_mode, WrapMode::Word { width: 0 });
    assert_eq!(pane.saved_wrap_mode, WrapMode::Word { width: 0 });
}

#[test]
fn typed_set_pane_wrap_mode_none_leaves_saved_wrapping() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=word").expect(":set pane wrap-mode=word failed");
    run_set(&mut ed, "pane wrap-mode=none").expect(":set pane wrap-mode=none failed");
    let pane = focused_pane(&ed);
    assert_eq!(pane.wrap_mode, WrapMode::None);
    // saved_wrap_mode must never collapse to None — it's still the restore
    // target for a future `:wrap` toggle-on.
    assert_eq!(pane.saved_wrap_mode, WrapMode::Word { width: 0 });
}

#[test]
fn typed_set_pane_unknown_setting_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "pane foo=bar");
    assert!(result.is_err(), "unknown setting must error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("unknown setting") && msg.contains("foo"),
        "error should name the unknown key: {msg}"
    );
}

/// A real setting that just isn't pane-eligible (`tab-width`'s `scope:` list
/// is `["global", "buffer"]`) must be rejected with its own valid scopes, not
/// silently accepted or confused with an unknown-setting error.
#[test]
fn typed_set_pane_ineligible_key_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "pane tab-width=2");
    assert!(result.is_err(), "pane-ineligible key must error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("tab-width") && msg.contains("global") && msg.contains("buffer"),
        "error should name the key and its actual valid scopes: {msg}"
    );
}

#[test]
fn typed_set_pane_language_errors() {
    let mut ed = editor_from("-[a]>b\n");
    // `language` is intercepted before pane-scope handling and has no
    // pane-scoped meaning — must still be a hard error, not a silent no-op.
    let result = run_set(&mut ed, "pane language=rust");
    assert!(result.is_err(), "pane-scoped language must error");
}

#[test]
fn typed_set_pane_invalid_wrap_mode_value_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "pane wrap-mode=bogus");
    assert!(result.is_err(), "invalid WrapMode value must error");
}

// ── `:wrap` toggle ───────────────────────────────────────────────────────────

/// Flip: before this fix, toggling on always hardcoded `Indent`, so this
/// would fail with `Soft { width: 0 } != Indent { width: 0 }`.
#[test]
fn wrap_toggle_restores_configured_mode_not_hardcoded_indent() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=soft").expect(":set pane wrap-mode=soft failed");

    ed.execute_typed("wrap", None).unwrap(); // off
    assert_eq!(focused_pane(&ed).wrap_mode, WrapMode::None);

    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(focused_pane(&ed).wrap_mode, WrapMode::Soft { width: 0 });
}

#[test]
fn wrap_toggle_after_set_pane_word_restores_word() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=word").expect(":set pane wrap-mode=word failed");

    ed.execute_typed("wrap", None).unwrap(); // off
    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(focused_pane(&ed).wrap_mode, WrapMode::Word { width: 0 });
}

/// A pane that was never explicitly configured falls back to `Indent` on
/// toggle-on — matching `Pane::new`'s default `saved_wrap_mode` derivation
/// for a `None` seed (see `hume-engine`'s `pane_new_none_seed_defaults_saved_wrap_mode_to_indent`).
#[test]
fn wrap_toggle_on_falls_back_to_indent_for_never_configured_pane() {
    let mut ed = editor_from("-[a]>b\n");
    {
        let pane = &mut ed.view.panes[ed.state.focused_pane_id];
        pane.wrap_mode = WrapMode::None;
        pane.saved_wrap_mode = WrapMode::Indent { width: 0 };
    }
    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(focused_pane(&ed).wrap_mode, WrapMode::Indent { width: 0 });
}

#[test]
fn wrap_toggle_on_zeroes_scroll_offsets() {
    let mut ed = editor_from("-[a]>b\n");
    {
        let pane = &mut ed.view.panes[ed.state.focused_pane_id];
        pane.wrap_mode = WrapMode::None;
        pane.viewport.horizontal_offset = 12;
        pane.viewport.top_row_offset = 3;
    }
    ed.execute_typed("wrap", None).unwrap(); // on
    let pane = focused_pane(&ed);
    assert_eq!(pane.viewport.horizontal_offset, 0);
    assert_eq!(pane.viewport.top_row_offset, 0);
}

// ── Split inheritance ────────────────────────────────────────────────────────

#[test]
fn split_inherits_source_panes_saved_wrap_mode() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=word").expect(":set pane wrap-mode=word failed");

    ed.execute_typed("split", None).unwrap();

    assert_eq!(focused_pane(&ed).wrap_mode, WrapMode::Word { width: 0 });
    assert_eq!(
        focused_pane(&ed).saved_wrap_mode,
        WrapMode::Word { width: 0 }
    );
}
