use super::*;
use crate::log::LogLevel;
use crate::test_support::SteelCtxTestHarness;

// ── Mode discriminant ─────────────────────────────────────────────────────

/// `new_init` sets `session = EvalSession::Init`.
///
/// Fail oracle: swap `EvalSession::Init` → `Runtime` in `new_init` → assert fires.
#[test]
fn new_init_has_init_session() {
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx_init();
    assert_eq!(
        ctx.session,
        EvalSession::Init,
        "new_init must set session = EvalSession::Init"
    );
}

/// `new_command` sets `session = EvalSession::Runtime`.
///
/// Fail oracle: swap `EvalSession::Runtime` → `Init` in `new_command` → assert fires.
#[test]
fn new_command_has_runtime_session() {
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx();
    assert_eq!(
        ctx.session,
        EvalSession::Runtime,
        "new_command must set session = EvalSession::Runtime"
    );
}

/// `new_activation` sets `session = EvalSession::Runtime` (same as command mode).
///
/// Runtime-activated plugin bodies use `new_activation` so `(call! …)` is
/// allowed inside them.  Fail oracle: set `session: EvalSession::Init` →
/// plugin bodies would be blocked from calling native commands.
#[test]
fn new_activation_has_runtime_session() {
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx_activation();
    assert_eq!(
        ctx.session,
        EvalSession::Runtime,
        "new_activation must set session = EvalSession::Runtime"
    );
}

/// `mode()` derives the correct `EvalMode` for all four `(session,
/// plugin_stack)` states. Independent oracle: expected variants come from
/// the truth table in `EvalMode`'s doc, not from `mode()`'s own logic.
///
/// Fail oracle: swap any two arms in `SteelCtx::mode`'s match → one of
/// these four assertions fires.
#[test]
fn mode_derives_from_session_and_plugin_stack() {
    use crate::attribution::PluginId;

    let mut h = SteelCtxTestHarness::new();
    assert_eq!(h.ctx_init().mode(), EvalMode::Init);
    assert_eq!(h.ctx().mode(), EvalMode::Command);

    h.plugin_stack
        .push(PluginId::parse("core:test-plugin").unwrap());
    assert_eq!(h.ctx_init().mode(), EvalMode::PluginLoad);
    assert_eq!(h.ctx_activation().mode(), EvalMode::PluginActivation);
}

// ── Terminal safety ───────────────────────────────────────────────────────

/// `new_command` reads `is_inline_output` off the host rather than
/// hardcoding it — `NullHost` (default) reports `false`.
///
/// Fail oracle: hardcode `is_inline_output: false` in `new_command` →
/// this assert fires even though the host says `true`.
#[test]
fn new_command_reads_inline_output_true_from_host() {
    use crate::null_host::InlineOutputHost;
    let mut host = InlineOutputHost::default();
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx_with_host(&mut host);
    assert!(
        ctx.is_inline_output,
        "new_command must read is_inline_output_command() from the host"
    );
}

/// The harness's default `NullHost` reports `is_inline_output_command() ==
/// false`, so a plain `ctx()` must carry `is_inline_output == false`.
#[test]
fn new_command_defaults_inline_output_false() {
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx();
    assert!(!ctx.is_inline_output, "NullHost must default to false");
}

// ── Focus snapshot (new_command) ──────────────────────────────────────────

/// `new_command` stores the focus IDs passed in; `live_focused_buffer_id`
/// starts equal to `focused_buffer_id`.
#[test]
fn new_command_stores_focus_ids() {
    let mut h = SteelCtxTestHarness::new();
    // ctx() uses PaneId::default() and BufferId::default() as the focus IDs.
    let ctx = h.ctx();
    assert_eq!(ctx.focused_pane_id, PaneId::default());
    assert_eq!(ctx.focused_buffer_id, BufferId::default());
    assert_eq!(
        ctx.live_focused_buffer_id, ctx.focused_buffer_id,
        "live_focused_buffer_id must start equal to focused_buffer_id"
    );
}

/// `new_command` stores `pending_char` correctly.
///
/// The harness passes `None`; test `new_command` directly for the `Some` case.
#[test]
fn new_command_stores_pending_char() {
    // Use ctx() (None) and verify the field is None.
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx();
    assert_eq!(ctx.pending_char, None, "default ctx has no pending_char");
}

// ── init mode: focus IDs are zeroed ───────────────────────────────────────

/// `new_init` leaves focus IDs at their defaults (not real buffer/pane IDs).
#[test]
fn new_init_focus_ids_are_default() {
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx_init();
    assert_eq!(ctx.focused_pane_id, PaneId::default());
    assert_eq!(ctx.focused_buffer_id, BufferId::default());
}

// ── log helper ────────────────────────────────────────────────────────────

/// `ctx.log(…)` appends to `pending_messages`.
///
/// Fail oracle: make `log` a no-op → pending_messages stays empty → assert fires.
#[test]
fn log_pushes_to_pending_messages() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        ctx.log(LogLevel::Info, "hello".into());
        ctx.log(LogLevel::Warning, "world".into());
    }
    assert_eq!(h.pending_messages.len(), 2);
    assert_eq!(h.pending_messages[0].0, LogLevel::Info);
    assert_eq!(h.pending_messages[1].0, LogLevel::Warning);
}

// ── Effect commit/rollback ────────────────────────────────────────────────

/// `pop_effect_marks(true)` marks every entry pushed since the mark as
/// committed, and leaves anything pushed before the mark untouched.
///
/// Fail oracle: mark all of `effects` (not just `[mark..]`) as committed
/// → the pre-mark entry ends up committed too → the first assert fires.
#[test]
fn pop_effect_marks_success_commits_marked_range() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        ctx.push_effect(Effect::GrammarSweep("before".into()));
        ctx.mark_effects();
        ctx.push_effect(Effect::GrammarSweep("after".into()));
        ctx.pop_effect_marks(true);
    }
    assert!(
        !h.effects[0].committed,
        "entry pushed before the mark must stay uncommitted"
    );
    assert!(
        h.effects[1].committed,
        "entry pushed after the mark must be committed on success"
    );
}

/// Nested marks: A1 (no mark), mark B, B1, mark C, C1/C2, pop(true) commits
/// C1/C2, B2, pop(false). B's own entries (B1, B2) are dropped but C1/C2 —
/// already committed by the nested activation that finished inside B —
/// survive B's failure, in their original order, alongside untouched A1.
///
/// Fail oracle: revert `pop_effect_marks`'s failure branch to
/// `self.effects.truncate(mark)` → C1/C2 vanish along with B1/B2 → the
/// log ends up `["a1"]` instead of `["a1", "c1", "c2"]`.
#[test]
fn pop_effect_marks_failure_keeps_committed_entries_in_order() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        ctx.push_effect(Effect::GrammarSweep("a1".into()));
        ctx.mark_effects(); // B begins
        ctx.push_effect(Effect::GrammarSweep("b1".into()));
        ctx.mark_effects(); // C begins, nested inside B
        ctx.push_effect(Effect::GrammarSweep("c1".into()));
        ctx.push_effect(Effect::GrammarSweep("c2".into()));
        ctx.pop_effect_marks(true); // C succeeds — commits c1, c2
        ctx.push_effect(Effect::GrammarSweep("b2".into()));
        ctx.pop_effect_marks(false); // B fails — drops b1/b2, keeps c1/c2
    }
    let names: Vec<&str> = h
        .effects
        .iter()
        .map(|e| match &e.effect {
            Effect::GrammarSweep(name) => name.as_str(),
            other => panic!("expected GrammarSweep, got {other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        vec!["a1", "c1", "c2"],
        "b1/b2 must be dropped, c1/c2 kept in original order, a1 untouched"
    );
    assert!(!h.effects[0].committed, "a1 was never inside a mark");
    assert!(
        h.effects[1].committed && h.effects[2].committed,
        "c1/c2 stay committed"
    );
}
