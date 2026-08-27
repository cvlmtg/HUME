use super::*;
use hume_grid::Rect;

use crate::editor::error::CommandError;
use hume_engine::pane::WrapMode;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn run_set(ed: &mut Editor, cmd: &str) -> Result<(), CommandError> {
    crate::editor::commands::typed_set(ed, Some(cmd), false)
}

fn focused_pane(ed: &Editor) -> &hume_engine::pane::Pane {
    &ed.view.panes[ed.state.focused_pane_id]
}

/// `pane`'s effective wrap mode, resolved pane → buffer → global — the same
/// path `Editor::focused_wrap_mode` uses, exposed here for panes other than
/// the focused one.
fn effective_wrap_mode(ed: &Editor, pid: hume_engine::pipeline::PaneId) -> WrapMode {
    let pane = &ed.view.panes[pid];
    let doc = ed.state.buffers.get(pane.buffer_id);
    crate::editor::commands::effective_wrap_mode(doc, &ed.state.settings, pane)
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
    assert_eq!(pane.wrap().mode, Some(WrapMode::Word { width: 0 }));
    assert_eq!(pane.wrap().saved, Some(WrapMode::Word { width: 0 }));
}

#[test]
fn typed_set_pane_wrap_mode_none_leaves_saved_wrapping() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=word").expect(":set pane wrap-mode=word failed");
    run_set(&mut ed, "pane wrap-mode=none").expect(":set pane wrap-mode=none failed");
    let pane = focused_pane(&ed);
    assert_eq!(pane.wrap().mode, Some(WrapMode::None));
    // saved_wrap_mode must never collapse to a non-wrapping value — it's
    // still the restore target for a future `:wrap` toggle-on.
    assert_eq!(pane.wrap().saved, Some(WrapMode::Word { width: 0 }));
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

// ── Scope precedence: pane → buffer → global ────────────────────────────────

/// The full three-rung chain in one test: global sets a baseline, a buffer
/// override supersedes it, and a pane pin supersedes that.
#[test]
fn wrap_mode_resolves_pane_over_buffer_over_global() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "global wrap-mode=none").expect("set global failed");
    assert_eq!(ed.focused_wrap_mode(), WrapMode::None, "no override yet");

    run_set(&mut ed, "buffer wrap-mode=soft").expect("set buffer failed");
    assert_eq!(
        ed.focused_wrap_mode(),
        WrapMode::Soft { width: 0 },
        "buffer override beats global"
    );

    run_set(&mut ed, "pane wrap-mode=word").expect("set pane failed");
    assert_eq!(
        ed.focused_wrap_mode(),
        WrapMode::Word { width: 0 },
        "pane override beats buffer"
    );
}

/// The user's actual story: a buffer-scoped wrap-mode (e.g. set from an
/// `on-language-set` hook) reaches a pane that was never explicitly told to
/// wrap one way or the other.
#[test]
fn set_buffer_wrap_mode_reaches_a_pane_with_no_override() {
    let mut ed = editor_from("-[a]>b\n");
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        None,
        "sanity: pane starts unpinned"
    );

    run_set(&mut ed, "buffer wrap-mode=word").expect("set buffer failed");
    assert_eq!(ed.focused_wrap_mode(), WrapMode::Word { width: 0 });
}

/// The other half: a pane explicitly pinned (`:set pane`/`:wrap`) keeps its
/// own style regardless of later buffer-scoped changes.
#[test]
fn set_buffer_wrap_mode_does_not_disturb_a_pinned_pane() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=none").expect("set pane failed");

    run_set(&mut ed, "buffer wrap-mode=word").expect("set buffer failed");
    assert_eq!(
        ed.focused_wrap_mode(),
        WrapMode::None,
        "explicit pane pin survives a buffer-scoped change"
    );
}

/// `:set global wrap-mode=…` is retroactive: it reaches a pane created
/// before the write, as long as that pane has no override of its own —
/// unlike the pre-buffer-scope behaviour, where global only seeded panes
/// created after the write.
#[test]
fn set_global_wrap_mode_is_retroactive_for_unpinned_panes() {
    let mut ed = editor_from("-[a]>b\n");
    assert_eq!(ed.focused_wrap_mode(), WrapMode::Indent { width: 0 });

    run_set(&mut ed, "global wrap-mode=none").expect("set global failed");
    assert_eq!(
        ed.focused_wrap_mode(),
        WrapMode::None,
        "global change reaches an already-open, unpinned pane"
    );
}

/// Two panes on the same buffer: one pinned, one not — the case the
/// pane-scope layer exists for. A buffer-scoped change reaches the
/// unpinned pane and skips the pinned one.
#[test]
fn set_buffer_wrap_mode_affects_only_the_unpinned_sibling() {
    let mut ed = editor_from("-[a]>b\n");
    let pid_a = ed.state.focused_pane_id;
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    // Focus is on B after the split — pin it explicitly. A stays unpinned.
    run_set(&mut ed, "pane wrap-mode=none").expect("set pane failed");

    run_set(&mut ed, "buffer wrap-mode=word").expect("set buffer failed");
    assert_eq!(
        effective_wrap_mode(&ed, pid_a),
        WrapMode::Word { width: 0 },
        "A: unpinned, follows the buffer override"
    );
    assert_eq!(
        effective_wrap_mode(&ed, pid_b),
        WrapMode::None,
        "B: pinned, unaffected"
    );
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
    assert_eq!(focused_pane(&ed).wrap().mode, Some(WrapMode::None));

    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        Some(WrapMode::Soft { width: 0 })
    );
}

#[test]
fn wrap_toggle_after_set_pane_word_restores_word() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=word").expect(":set pane wrap-mode=word failed");

    ed.execute_typed("wrap", None).unwrap(); // off
    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        Some(WrapMode::Word { width: 0 })
    );
}

/// A pane that was never explicitly configured (no pane pin, and the
/// buffer/global setting it's inheriting doesn't wrap) falls back to
/// `Indent` on toggle-on — `:wrap` must always visibly wrap, never silently
/// no-op just because there was nothing to restore. `Indent` is the
/// last-resort fallback here specifically because the global itself is
/// `none` — there is no configured style to reach for instead.
#[test]
fn wrap_toggle_on_falls_back_to_indent_for_never_configured_pane() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "global wrap-mode=none").expect("set global failed");
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        None,
        "sanity: pane is unpinned"
    );

    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        Some(WrapMode::Indent { width: 0 })
    );
}

/// The other half of the fallback: when the global itself is configured to a
/// wrapping style, toggle-on must reach for *that* — not the hardcoded
/// `Indent` default — for a pane whose inherited mode doesn't wrap (here, a
/// buffer override pins `none` while global says `word`).
#[test]
fn wrap_toggle_on_falls_back_to_the_configured_global_style_not_indent() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "global wrap-mode=word").expect("set global failed");
    run_set(&mut ed, "buffer wrap-mode=none").expect("set buffer failed");
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        None,
        "sanity: pane is unpinned"
    );
    assert_eq!(
        ed.focused_wrap_mode(),
        WrapMode::None,
        "sanity: inherited (buffer-overridden) mode doesn't wrap"
    );

    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        Some(WrapMode::Word { width: 0 }),
        "falls back to the configured global style, not the hardcoded Indent default"
    );
}

/// `:wrap` off then on, with no `:set pane` in between, restores
/// *inheritance* — not a frozen snapshot of whatever the buffer/global
/// setting happened to resolve to at toggle-off time. The pane keeps
/// following later buffer-scoped changes.
///
/// Fail oracle: if `saved_wrap_mode` stored the resolved mode instead of the
/// override to restore, the pane would stay pinned to `word` (the mode it
/// was wrapping with at toggle-off) and this assertion would fail.
#[test]
fn wrap_toggle_off_then_on_restores_inheritance_not_a_pin() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "buffer wrap-mode=word").expect("set buffer failed");
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        None,
        "sanity: unpinned, inheriting the buffer override"
    );

    ed.execute_typed("wrap", None).unwrap(); // off
    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        None,
        "toggling back on restores inheritance, not a pin to \"word\""
    );

    run_set(&mut ed, "buffer wrap-mode=soft").expect("set buffer failed");
    assert_eq!(
        ed.focused_wrap_mode(),
        WrapMode::Soft { width: 0 },
        "the pane followed the later buffer-scoped change"
    );
}

/// The other half: `:wrap` off then on, when the pane *was* explicitly
/// pinned via `:set pane`, restores that exact pin — not inheritance.
#[test]
fn wrap_toggle_off_then_on_restores_an_explicit_pane_pin() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "buffer wrap-mode=word").expect("set buffer failed");
    run_set(&mut ed, "pane wrap-mode=soft").expect("set pane failed");

    ed.execute_typed("wrap", None).unwrap(); // off
    ed.execute_typed("wrap", None).unwrap(); // on
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        Some(WrapMode::Soft { width: 0 }),
        "toggling back on restores the explicit pin, ignoring the buffer's \"word\""
    );
}

/// A wrap-mode change zeroes horizontal scroll (meaningless once wrapped) but
/// leaves `top_row_offset` alone — see `pane_state::toggle_focused_wrap`'s doc.
#[test]
fn wrap_toggle_on_zeroes_horizontal_offset_only() {
    let mut ed = editor_from("-[a]>b\n");
    {
        let pane = &mut ed.view.panes[ed.state.focused_pane_id];
        pane.set_wrap(hume_engine::pane::WrapOverride {
            mode: Some(WrapMode::None),
            saved: None,
        });
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

/// `:set pane wrap-mode=…`'s half of the same horizontal-offset rule
/// (`set_focused_wrap_override`, forked from `toggle_focused_wrap`): zeroes
/// horizontal scroll when the pin actually changes the pane's *effective*
/// mode.
#[test]
fn set_pane_wrap_mode_zeroes_horizontal_offset_on_an_effective_change() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "global wrap-mode=none").expect("set global failed");
    ed.viewport_mut().horizontal_offset = 12;

    run_set(&mut ed, "pane wrap-mode=soft").expect(":set pane wrap-mode=soft failed");
    assert_eq!(
        focused_pane(&ed).viewport.horizontal_offset,
        0,
        "none → soft is an effective-mode change, so horizontal scroll is zeroed"
    );
}

/// The other half: pinning a pane to the mode it already effectively has
/// (no visible change) must *not* zero horizontal scroll — the offset stays
/// meaningful because the pane never stopped being unwrapped.
#[test]
fn set_pane_wrap_mode_leaves_horizontal_offset_when_effective_mode_is_unchanged() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "global wrap-mode=none").expect("set global failed");
    ed.viewport_mut().horizontal_offset = 12;

    run_set(&mut ed, "pane wrap-mode=none").expect(":set pane wrap-mode=none failed");
    assert_eq!(
        focused_pane(&ed).viewport.horizontal_offset,
        12,
        "none → none is not an effective-mode change, so horizontal scroll survives"
    );
}

/// Turning wrap *off* must not force-reset `top_row_offset`: it addresses a
/// row inside `top_line`'s block in either wrap mode (`scroll::set_top`
/// writes it unconditionally). If the new (no-wrap) block is shorter than
/// the old one, the offset is now stale — but `scroll::clamp_viewport_top`
/// repairs that once per pane per frame, not `toggle_focused_wrap` itself,
/// so the raw value must survive the `:set` call untouched.
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
    assert_eq!(pane.wrap().mode, Some(WrapMode::None));
    assert_eq!(
        pane.viewport.top_row_offset, 3,
        "toggle_focused_wrap itself must not reset a still-unvalidated offset"
    );

    // No-wrap: line 0's whole block is 1 row (content only, no providers
    // registered) — the only valid address is row 0, so the next frame's
    // self-heal must pull the stale offset down to it.
    ed.render_to_buf(Rect::new(0, 0, 40, 8));
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
    assert_eq!(pane.wrap().mode, Some(WrapMode::Soft { width: 20 }));
    assert_eq!(
        pane.viewport.top_row_offset, 3,
        "the raw offset survives the width change untouched"
    );

    ed.render_to_buf(Rect::new(0, 0, 40, 8));
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
    impl hume_engine::providers::DecorationSource for ThreeBeforeLine0 {
        fn kinds(&self) -> hume_engine::providers::DecorationKinds {
            hume_engine::providers::DecorationKinds::VIRTUAL_LINE
        }
        fn decorations_for_line(
            &self,
            line_idx: usize,
            out: &mut Vec<hume_engine::providers::Decoration>,
        ) {
            if line_idx == 0 {
                for _ in 0..3 {
                    out.push(hume_engine::providers::Decoration::VirtualLine(
                        hume_engine::providers::VirtualLine {
                            anchor: hume_engine::providers::VirtualLineAnchor::Before(0),
                            provider_id: 0,
                            text: "V".to_string(),
                            segments: Vec::new(),
                            base_scope: None,
                        },
                    ));
                }
            }
        }
    }

    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=soft").expect(":set pane wrap-mode=soft failed");
    ed.view.panes[ed.state.focused_pane_id]
        .providers
        .add_decoration_source(Box::new(ThreeBeforeLine0));
    {
        let pane = &mut ed.view.panes[ed.state.focused_pane_id];
        pane.viewport.top_line = 0;
        pane.viewport.top_row_offset = 1; // inside the Before(0) block
    }

    ed.execute_typed("wrap", None).unwrap(); // off
    let pane = focused_pane(&ed);
    assert_eq!(pane.wrap().mode, Some(WrapMode::None));
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

    assert_eq!(
        focused_pane(&ed).wrap().mode,
        Some(WrapMode::Word { width: 0 })
    );
    assert_eq!(
        focused_pane(&ed).wrap().saved,
        Some(WrapMode::Word { width: 0 })
    );
}

// ── Render path ───────────────────────────────────────────────────────────────

/// The render path's own resolve call (`Editor::resolve_pane_settings`, which
/// `frame.rs` renders through) must apply the same pane → buffer → global
/// precedence `effective_wrap_mode` does elsewhere, not a two-rung
/// pane → global shortcut that skips the buffer rung. Asserting a
/// `WrapMode::None` result (no width to resolve) catches a dropped buffer
/// rung directly — unlike assertions that read the resolver's own return
/// value, which would keep passing even if `frame.rs` stopped calling it.
#[test]
fn resolve_pane_settings_honours_the_buffer_rung() {
    let mut ed = editor_from("-[a]>b\n");
    // Global default (Indent) wraps; only a buffer override can produce None.
    run_set(&mut ed, "buffer wrap-mode=none").expect("set buffer failed");
    let pid = ed.state.focused_pane_id;
    let (settings, _gutter_w) = ed.resolve_pane_settings(pid);
    assert_eq!(
        settings.wrap_mode,
        WrapMode::None,
        "the render path must resolve the buffer override, not just pane → global"
    );
}

// ── Per-(pane, buffer) memory ────────────────────────────────────────────────
//
// A pane's wrap pin (`:wrap`/`:set pane`) lives in `Pane::wraps`, keyed by
// buffer — the same lifetime `saved_scrolls` already gives scroll position.
// This is what lets a per-filetype `on-language-set` default reach a pane
// that toggled wrap off in an earlier, unrelated buffer.

fn open_second_buffer(ed: &mut Editor) -> BufferId {
    let text = BufferText::from("other buffer\n");
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(0));
    let bid = ed.open_buffer(Buffer::new(text, sels));
    ed.switch_to_buffer_with_jump(bid);
    bid
}

/// `:wrap` off pins the pane for the buffer it was toggled in. Switching to
/// another buffer in the same pane must not carry that pin along — the new
/// buffer resolves through its own buffer/global chain, which is exactly
/// what lets a per-language `on-language-set` default apply there.
#[test]
fn wrap_off_pin_does_not_follow_a_buffer_switch() {
    let mut ed = editor_from("-[a]>b\n");
    let bid_first = ed.focused_buffer_id();
    ed.execute_typed("wrap", None).unwrap(); // off
    assert_eq!(focused_pane(&ed).wrap().mode, Some(WrapMode::None));

    open_second_buffer(&mut ed);
    assert_ne!(
        ed.focused_buffer_id(),
        bid_first,
        "sanity: switched buffers"
    );
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        None,
        "the new buffer is unpinned — it did not inherit the old buffer's off pin"
    );
    assert_eq!(
        ed.focused_wrap_mode(),
        WrapMode::Indent { width: 0 },
        "the new buffer resolves through its own buffer/global chain"
    );
}

/// Switching back to the first buffer restores its pin — the pane remembers
/// it, it doesn't just forget it forever on switch-away.
#[test]
fn wrap_off_pin_is_restored_on_switching_back() {
    let mut ed = editor_from("-[a]>b\n");
    let bid_first = ed.focused_buffer_id();
    ed.execute_typed("wrap", None).unwrap(); // off
    assert_eq!(focused_pane(&ed).wrap().mode, Some(WrapMode::None));

    open_second_buffer(&mut ed);
    ed.switch_to_buffer_with_jump(bid_first);
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        Some(WrapMode::None),
        "switching back to the first buffer restores its pin"
    );
}

/// `:set pane wrap-mode=…` pins for the buffer it was set on — it does not
/// leak into a different buffer shown by the same pane.
#[test]
fn set_pane_wrap_mode_pin_does_not_leak_to_another_buffer() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "pane wrap-mode=word").expect(":set pane wrap-mode=word failed");

    open_second_buffer(&mut ed);
    assert_eq!(
        focused_pane(&ed).wrap().mode,
        None,
        "a pin set on one buffer does not apply to a different buffer"
    );
}

/// Closing a buffer drops the pane's wrap-mode memory for it — the same
/// cleanup `forget_buffer` already does for `saved_scrolls`.
#[test]
fn closing_a_buffer_drops_its_wrap_override() {
    let mut ed = editor_from("-[a]>b\n");
    let bid_first = ed.focused_buffer_id();
    ed.execute_typed("wrap", None).unwrap(); // off, pins bid_first
    let bid_second = open_second_buffer(&mut ed);

    ed.close_buffer(bid_first);
    assert_eq!(
        ed.focused_buffer_id(),
        bid_second,
        "sanity: closing bid_first leaves the pane on the other open buffer"
    );

    let pid = ed.state.focused_pane_id;
    assert!(
        !ed.view.panes[pid].wraps.contains_key(bid_first),
        "the closed buffer's wrap override is dropped, not leaked in the map forever"
    );
}
