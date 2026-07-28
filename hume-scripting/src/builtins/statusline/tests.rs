use super::*;
use crate::test_support::SteelCtxTestHarness;
use steel::rvals::IntoSteelVal as _;

fn empty_list() -> SteelVal {
    Vec::<SteelVal>::new().into_steelval().unwrap()
}

fn string_list(items: &[&str]) -> SteelVal {
    items
        .iter()
        .map(|s| SteelVal::StringV((*s).into()))
        .collect::<Vec<_>>()
        .into_steelval()
        .unwrap()
}

// ── Eval-mode gating ───────────────────────────────────────────────────────

/// `configure-statusline!` is registered `open` (`builtins/mod.rs`) — no
/// eval-mode gate at all, since it writes the same `EditorSettings.statusline`
/// field as `set-option!` (also `open`) through the same
/// `editor::settings_ops::apply_global` chokepoint regardless of caller.
/// Reaches the host from ordinary command-mode context.
///
/// Fail oracle: change `configure-statusline!`'s table entry back to
/// `config` → this call would fail with a gate error instead of reaching
/// (and erroring on) `NullHost`.
#[test]
fn configure_statusline_reaches_host_from_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx(); // EvalMode::Command
    let left = string_list(&["FileName"]);
    let result = configure_statusline(&mut ctx, left, empty_list(), empty_list());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("only valid during") && !msg.contains("command body"),
        "must reach the host, not a gate; got: {msg}"
    );
}

// ── Type validation of section args ───────────────────────────────────────

/// `configure-statusline!` rejects a non-list `left` argument.
///
/// Fail oracle: remove the list check → a boolean would be accepted and the
/// iterator would produce garbage.
#[test]
fn configure_statusline_non_list_left_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = configure_statusline(
        &mut ctx,
        SteelVal::BoolV(false), // not a list
        empty_list(),
        empty_list(),
    );
    assert!(
        result.is_err(),
        "configure-statusline! must reject non-list left"
    );
}

/// `configure-statusline!` rejects a non-list `center` argument.
#[test]
fn configure_statusline_non_list_center_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = configure_statusline(&mut ctx, empty_list(), SteelVal::IntV(42), empty_list());
    assert!(
        result.is_err(),
        "configure-statusline! must reject non-list center"
    );
}

/// `configure-statusline!` rejects a non-list `right` argument.
#[test]
fn configure_statusline_non_list_right_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = configure_statusline(
        &mut ctx,
        empty_list(),
        empty_list(),
        SteelVal::SymbolV("bad".into()),
    );
    assert!(
        result.is_err(),
        "configure-statusline! must reject non-list right"
    );
}

/// `configure-statusline!` rejects a list that contains a non-string element.
///
/// Fail oracle: remove the element type check → integer elements would be
/// passed as-is to the host and likely panic or produce garbage element names.
#[test]
fn configure_statusline_non_string_list_item_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let bad_list: SteelVal = vec![SteelVal::IntV(1)].into_steelval().unwrap();
    let result = configure_statusline(&mut ctx, bad_list, empty_list(), empty_list());
    assert!(
        result.is_err(),
        "configure-statusline! must reject non-string list items"
    );
}

// ── Guard passes, host called ─────────────────────────────────────────────

/// In init mode with valid args, `configure-statusline!` reaches the host.
/// NullHost returns Err, proving type validation passed and the call landed.
#[test]
fn configure_statusline_init_mode_calls_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let left = string_list(&["FileName"]);
    let result = configure_statusline(&mut ctx, left, empty_list(), empty_list());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("only valid during"),
        "must reach the host, not the guard; got: {msg}"
    );
}
