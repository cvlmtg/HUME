use super::*;
use crate::test_support::SteelCtxTestHarness;
use hume_engine::pipeline::BufferId;

fn default_bid() -> BidArg {
    BidArg(BufferId::default())
}

// ── Gate (init mode rejection) ────────────────────────────────────────────
//
// Every builtin below is `cmd`-gated in `builtins!`'s registration table —
// the gate lives in the registration wrapper closure, not the function
// body, so these test the gate primitive directly rather than calling the
// builtin (its body has no guard to hit).

/// `current-buffer` is blocked in init mode.
///
/// Fail oracle: change `current-buffer`'s table entry from `cmd` to
/// `open` → `focused_buffer_id` (which is Default in init) would be
/// returned, silently giving wrong data.
#[test]
fn current_buffer_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_cmd(&h.ctx_init(), "current-buffer");
    assert!(result.is_err(), "current-buffer must error in init mode");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("init"),
        "error must mention 'init'; got: {msg}"
    );
}

/// `current-pane` is blocked in init mode.
#[test]
fn current_pane_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "current-pane").is_err());
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

/// `set-buffer-language!` is blocked in init mode.
#[test]
fn set_buffer_language_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_cmd(&h.ctx_init(), "set-buffer-language!");
    assert!(
        result.is_err(),
        "set-buffer-language! must error in init mode"
    );
}

/// `current-line-number` is blocked in init mode.
#[test]
fn current_line_number_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "current-line-number").is_err());
}

/// `current-selections` is blocked in init mode.
#[test]
fn current_selections_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "current-selections").is_err());
}

/// `char-index->line` is blocked in init mode.
#[test]
fn char_index_to_line_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "char-index->line").is_err());
}

// ── Type errors (wrong arg type) ──────────────────────────────────────────
//
// `buffer-path`/`buffer-name`/`buffer-dirty?` don't decode `bid` in-body
// (it's a typed `BidArg` param) — that decode-failure path is covered
// once, centrally, by `args::tests::bid_arg_rejects_non_buffer_id`.

/// `char-index->line` rejects a non-integer and a negative integer argument.
///
/// Fail oracle: remove the `n >= 0` guard → `IntV(-1)` would be accepted and
/// cast to a huge `usize`, silently corrupting the lookup instead of erroring.
#[test]
fn char_index_to_line_wrong_type_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = char_index_to_line(&mut ctx, SteelVal::StringV("not-an-int".into()));
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("expected a non-negative integer")
    );

    let mut ctx = h.ctx();
    let result = char_index_to_line(&mut ctx, SteelVal::IntV(-1));
    assert!(result.is_err());
}

// ── Invalid buffer ID (NullHost always returns buffer_exists=false) ───────

/// `buffer-path` with a valid BufferId but non-existent buffer raises an error.
///
/// Fail oracle: remove the `buffer_exists` check → `buffer_path` is called
/// on a nonexistent buffer, which could panic or return garbage.
#[test]
fn buffer_path_invalid_id_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    // NullHost.buffer_exists always returns false.
    let result = buffer_path(&mut ctx, default_bid());
    assert!(result.is_err(), "non-existent buffer id must error");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("invalid buffer id")
    );
}

// ── Command-mode success paths (NullHost read methods return None/empty) ──

/// `current-buffer` in command mode returns the focused buffer id as a SteelVal.
///
/// Fail oracle: return a hardcoded or wrong id → the assert on the type fires.
#[test]
fn current_buffer_command_mode_returns_steel_buffer_id() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = current_buffer(&mut ctx);
    assert!(
        result.is_ok(),
        "current-buffer must succeed in command mode"
    );
    // Must return a Custom value (SteelBufferId is opaque).
    assert!(
        matches!(result.unwrap(), SteelVal::Custom(_)),
        "current-buffer must return a SteelVal::Custom (BufferId)"
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

/// `current-line-number` returns `#f` when the host has no cursor (NullHost).
#[test]
fn current_line_number_returns_false_when_no_cursor() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = current_line_number(&mut ctx);
    assert!(matches!(result, Ok(SteelVal::BoolV(false))));
}

/// `current-selections` returns `#f` when the host has no pane state (NullHost).
#[test]
fn current_selections_returns_false_when_no_pane_state() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = current_selections(&mut ctx);
    assert!(matches!(result, Ok(SteelVal::BoolV(false))));
}

/// `char-index->line` returns `#f` when the host has no pane state (NullHost).
#[test]
fn char_index_to_line_returns_false_when_no_pane_state() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = char_index_to_line(&mut ctx, SteelVal::IntV(0));
    assert!(matches!(result, Ok(SteelVal::BoolV(false))));
}
