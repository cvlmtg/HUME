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
// A single data-driven check produces every "unknown setting" / "wrong
// scope for this setting" message, instead of two divergent hardcoded
// strings.

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

/// `SetCompleter` tolerates a stray double space before the key (a6e5adc), so
/// `typed_set` must accept the same input on Enter — otherwise Tab-completing
/// through a double space produces a command line that errors.
#[test]
fn typed_set_tolerates_double_space_before_key() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "global  tab-width=2");
    assert!(
        result.is_ok(),
        "double space before key must not error: {:?}",
        result.err().map(|e| e.message().to_owned())
    );
    assert_eq!(ed.state.settings.tab_width, 2);
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

/// Flip: toggling `:wrap` back on must restore the configured mode, not
/// hardcode `Indent` — a hardcoded toggle would fail here with
/// `Soft { width: 0 } != Indent { width: 0 }`.
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

/// A wrap-mode change zeroes horizontal scroll (meaningless once wrapped) but
/// leaves `top_row_offset` alone — see `apply_focused_wrap_mode`'s doc.
#[test]
fn wrap_toggle_on_zeroes_horizontal_offset_only() {
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
    assert_eq!(
        pane.viewport.top_row_offset, 3,
        "top_row_offset is a row address valid in either wrap mode — a mode \
         change must not discard it"
    );
}

/// Turning wrap *off* must not force-reset `top_row_offset`: it addresses a
/// row inside `top_line`'s block in either wrap mode (`scroll::set_top`
/// writes it unconditionally). If the new (no-wrap) block is shorter than
/// the old one, the offset is now stale — but `scroll::clamp_viewport_top`
/// repairs that once per pane per frame, not `apply_focused_wrap_mode`
/// itself, so the raw value must survive the `:set` call untouched.
#[test]
fn wrap_toggle_off_leaves_top_row_offset_for_the_next_frame_to_clamp() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=soft").expect(":set pane wrap-mode=soft failed");
    {
        let pane = &mut ed.view.panes[ed.state.focused_pane_id];
        pane.viewport.top_row_offset = 3;
    }
    ed.execute_typed("wrap", None).unwrap(); // off
    let pane = focused_pane(&ed);
    assert_eq!(pane.wrap_mode, WrapMode::None);
    assert_eq!(
        pane.viewport.top_row_offset, 3,
        "apply_focused_wrap_mode itself must not reset a still-unvalidated offset"
    );

    // No-wrap: line 0's whole block is 1 row (content only, no providers
    // registered) — the only valid address is row 0, so the next frame's
    // self-heal must pull the stale offset down to it.
    ed.render_to_buf(ratatui::layout::Rect::new(0, 0, 40, 8));
    assert_eq!(
        focused_pane(&ed).viewport.top_row_offset,
        0,
        "clamp_viewport_top, not the wrap-mode change, is what repairs staleness"
    );
}

/// Changing the wrap style/width while already wrapping (`:set pane
/// wrap-mode=` to a different variant) must likewise leave `top_row_offset`
/// for `clamp_viewport_top` to repair, not reset it inline — the old offset
/// was measured against the previous width and may no longer be a valid
/// sub-row index once the width changes.
#[test]
fn set_pane_wrap_mode_change_while_wrapping_leaves_top_row_offset_for_the_next_frame_to_clamp() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=soft:80").expect(":set pane wrap-mode=soft:80 failed");
    {
        let pane = &mut ed.view.panes[ed.state.focused_pane_id];
        pane.viewport.top_row_offset = 3;
    }
    run_set(&mut ed, "pane wrap-mode=soft:20").expect(":set pane wrap-mode=soft:20 failed");
    let pane = focused_pane(&ed);
    assert_eq!(pane.wrap_mode, WrapMode::Soft { width: 20 });
    assert_eq!(
        pane.viewport.top_row_offset, 3,
        "the raw offset survives the width change untouched"
    );

    ed.render_to_buf(ratatui::layout::Rect::new(0, 0, 40, 8));
    assert_eq!(
        focused_pane(&ed).viewport.top_row_offset,
        0,
        "line 0's block is 1 row under either width here, so clamp pulls the stale offset to it"
    );
}

/// The scenario the fix is actually for: a `Before` block on the top line
/// that the pre-fix reset would blow past. Wrap on, scrolled so `top_line`
/// sits inside a 3-row `Before(0)` block (`top_row_offset = 1`, one row
/// already scrolled past, two still showing); `:set wrap-mode=none` must not
/// jump the viewport back up to the top of that block — the address is
/// still valid (a `Before` block occupies the same rows regardless of wrap
/// mode) and clamp_viewport_top would find nothing to repair.
#[test]
fn wrap_toggle_off_does_not_discard_a_still_valid_offset_inside_a_before_block() {
    struct ThreeBeforeLine0;
    impl hume_engine::providers::VirtualLineSource for ThreeBeforeLine0 {
        fn virtual_lines(
            &self,
            visible: std::ops::Range<usize>,
            _content_width: u16,
            out: &mut Vec<hume_engine::providers::VirtualLine>,
        ) {
            if visible.contains(&0) {
                for _ in 0..3 {
                    out.push(hume_engine::providers::VirtualLine {
                        anchor: hume_engine::providers::VirtualLineAnchor::Before(0),
                        provider_id: 0,
                        text: "V".to_string(),
                        segments: Vec::new(),
                    });
                }
            }
        }
    }

    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=soft").expect(":set pane wrap-mode=soft failed");
    ed.view.panes[ed.state.focused_pane_id]
        .providers
        .add_virtual_line_source(Box::new(ThreeBeforeLine0));
    {
        let pane = &mut ed.view.panes[ed.state.focused_pane_id];
        pane.viewport.top_line = 0;
        pane.viewport.top_row_offset = 1; // inside the Before(0) block
    }

    ed.execute_typed("wrap", None).unwrap(); // off
    let pane = focused_pane(&ed);
    assert_eq!(pane.wrap_mode, WrapMode::None);
    assert_eq!(
        pane.viewport.top_row_offset, 1,
        "still-valid address inside the Before block must not be discarded"
    );
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
