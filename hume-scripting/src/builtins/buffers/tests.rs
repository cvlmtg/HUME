use super::*;
use crate::test_support::{SteelCtxTestHarness, default_pane};

// ── Gate (init mode rejection) ────────────────────────────────────────────
//
// Every builtin below is `cmd`-gated in `builtins!`'s registration table:
// the gate lives in the registration wrapper closure, not the function
// body, so these test the gate primitive directly rather than calling the
// builtin (its body has no guard to hit).

/// `focused-pane` is blocked in init mode: no meaningful focus exists yet.
///
/// An `open` table entry would return `PaneId::default()` during init, when
/// there is no live host to read, silently giving wrong data.
#[test]
fn focused_pane_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "focused-pane").is_err());
}

/// `buffers` is blocked in init mode.
#[test]
fn buffers_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffers").is_err());
}

/// `panes` is blocked in init mode.
#[test]
fn panes_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "panes").is_err());
}

/// `buffer-path` is blocked in init mode.
#[test]
fn buffer_path_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-path").is_err());
}

/// `buffer-display-path` is blocked in init mode.
#[test]
fn buffer_display_path_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-display-path").is_err());
}

/// `buffer-name` is blocked in init mode.
#[test]
fn buffer_name_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-name").is_err());
}

/// `buffer-dirty?` is blocked in init mode.
#[test]
fn buffer_dirty_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-dirty?").is_err());
}

/// `close-buffer!` is blocked in init mode.
#[test]
fn close_buffer_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "close-buffer!").is_err());
}

/// `switch-to-buffer!` is blocked in init mode.
#[test]
fn switch_to_buffer_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "switch-to-buffer!").is_err());
}

/// `buffer-cursor-line` is blocked in init mode.
#[test]
fn buffer_cursor_line_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-cursor-line").is_err());
}

/// `buffer-selections` is blocked in init mode.
#[test]
fn buffer_selections_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-selections").is_err());
}

/// `offset->line` is blocked in init mode.
#[test]
fn offset_to_line_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "offset->line").is_err());
}

/// `buffer-text` is blocked in init mode.
#[test]
fn buffer_text_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-text").is_err());
}

/// `%buffer-lines` (the Rust half of `buffer-lines`) is blocked in init mode.
#[test]
fn buffer_lines_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "%buffer-lines").is_err());
}

/// `buffer-line-count` is blocked in init mode.
#[test]
fn buffer_line_count_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-line-count").is_err());
}

/// `line->offset` is blocked in init mode.
#[test]
fn line_to_offset_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "line->offset").is_err());
}

// `offset->line`/`%buffer-lines`/`line->offset`'s wrong-type-argument
// checks are covered centrally, by `args::tests`' own unit tests on the
// `Usize`/`OptUsize` `FromSteelVal` newtypes their `builtins!` table
// entries declare. That decode happens at Steel's own registration
// boundary, before any of these functions' bodies (which take a plain
// `usize`/`Option<usize>`) ever run, so it cannot be exercised by
// calling the function directly with a malformed `SteelVal`.
//
// Likewise, every explicit-`pane` builtin's "invalid buffer id" error on a
// closed buffer is raised by `args::LivePane`'s `BuiltinArg::resolve`,
// in the `builtins!`-registered closure, before the function body (which
// takes a plain `PaneHandle`, already known live) ever runs. A direct
// call here has no way to reach that check at all; it's covered once,
// centrally, through a real `ScriptingHost` by
// `builtins::tests::live_pane_builtins_raise_on_a_closed_buffer_through_real_registration`.

// ── Command-mode success paths (NullHost read methods return None/empty) ──

/// `focused-pane` reaches `ctx.host.buffers()` (the live host read),
/// proven by NullHost's `PaneHandle::buffer_only(BufferId::default())`
/// return round-tripping through `SteelPane`.
///
/// A stale cached snapshot would still pass this assertion, since both hold
/// the same default on a fresh harness. The live read is guaranteed by the
/// function body calling `ctx.host.buffers()`. See
/// `wire_response_decodes_with_the_requesting_buffers_encoding_not_live_focus`
/// (`hume-editor`) for a case where the two genuinely diverge.
#[test]
fn focused_pane_command_mode_returns_steel_pane_id() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = focused_pane(&mut ctx);
    assert!(result.is_ok(), "focused-pane must succeed in command mode");
    assert!(
        matches!(result.unwrap(), SteelVal::Custom(_)),
        "focused-pane must return a SteelVal::Custom (PaneId)"
    );
}

/// `buffers` in command mode returns an empty list (NullHost has no buffers).
#[test]
fn buffers_command_mode_returns_empty_list() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = buffers(&mut ctx);
    assert!(result.is_ok());
    assert!(
        matches!(result.unwrap(), SteelVal::ListV(lst) if lst.is_empty()),
        "buffers must return an empty list when no buffers exist"
    );
}

/// `buffer-cursor-line` raises when `pane` carries no pane state (NullHost).
/// Kind-B fail-fast: a builtin that needs a pane to answer meaningfully
/// must say so loudly when it doesn't have one, not silently guess a
/// default (see `CursorHost`'s doc).
#[test]
fn buffer_cursor_line_raises_with_no_pane_state() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = buffer_cursor_line(&mut ctx, default_pane());
    assert!(result.is_err());
}

/// `buffer-selections` raises when `pane` carries no pane state (NullHost).
#[test]
fn buffer_selections_raises_with_no_pane_state() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = buffer_selections(&mut ctx, default_pane());
    assert!(result.is_err());
}
