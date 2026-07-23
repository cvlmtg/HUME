use super::*;
use crate::test_support::SteelCtxTestHarness;

fn make_list(vals: Vec<SteelVal>) -> SteelVal {
    SteelVal::ListV(vals.into_iter().collect())
}

/// NullHost returns `Ok(false)` for `command_is_native` (no registry) →
/// "not a native command" path → error logged.
#[test]
fn call_bang_unknown_command_logs_error() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        call_command_primitive(
            &mut ctx,
            "plum-ensure-grammars".to_string(),
            make_list(vec![]),
        )
        .unwrap();
    }
    assert!(
        h.pending_messages
            .iter()
            .any(|(level, msg)| *level == LogLevel::Error && msg.contains("plum-ensure-grammars")),
        "unknown command must log an error; got: {:?}",
        h.pending_messages
    );
}

#[test]
fn call_bang_unknown_command_command_mode_logs_error() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        call_command_primitive(&mut ctx, "move-right".to_string(), make_list(vec![])).unwrap();
    }
    assert!(
        h.pending_messages
            .iter()
            .any(|(level, msg)| *level == LogLevel::Error && msg.contains("move-right")),
        "unknown command in command mode must log an error; got: {:?}",
        h.pending_messages
    );
}

#[test]
fn call_bang_multiple_unknown_commands_each_log_error() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        call_command_primitive(&mut ctx, "move-right".to_string(), make_list(vec![])).unwrap();
        call_command_primitive(&mut ctx, "move-left".to_string(), make_list(vec![])).unwrap();
    }
    let has_right = h
        .pending_messages
        .iter()
        .any(|(level, msg)| *level == LogLevel::Error && msg.contains("move-right"));
    let has_left = h
        .pending_messages
        .iter()
        .any(|(level, msg)| *level == LogLevel::Error && msg.contains("move-left"));
    assert!(
        has_right && has_left,
        "each unknown command must produce an error"
    );
}

#[test]
fn request_wait_char_outside_invocation_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Init;
    let err = super::super::errors::require_cmd(&ctx, "request-wait-char!").unwrap_err();
    assert!(
        err.to_string().contains("not available during init"),
        "got: {err}"
    );
}

#[test]
fn request_wait_char_stores_cmd() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    request_wait_char(&mut ctx, "replace".to_string()).unwrap();
    assert_eq!(ctx.wait_char_request, Some("replace".to_string()));
}

#[test]
fn pending_char_returns_false_when_none() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = pending_char(&mut ctx).unwrap();
    assert_eq!(result, SteelVal::BoolV(false));
}

#[test]
fn pending_char_returns_string_when_set() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.pending_char = Some('(');
    let result = pending_char(&mut ctx).unwrap();
    assert_eq!(result, SteelVal::StringV("(".into()));
}

// ── parse_count_extend direct tests ──────────────────────────────────────

#[test]
fn parse_count_extend_empty_gives_defaults() {
    assert_eq!(parse_count_extend(&[]).unwrap(), (Some(1), false));
}

#[test]
fn parse_count_extend_count_only() {
    assert_eq!(
        parse_count_extend(&[SteelVal::IntV(5)]).unwrap(),
        (Some(5), false)
    );
}

#[test]
fn parse_count_extend_count_and_extend() {
    assert_eq!(
        parse_count_extend(&[SteelVal::IntV(3), SteelVal::BoolV(true)]).unwrap(),
        (Some(3), true)
    );
}

/// Negative counts clamp to `Some(1)` — same as a native keypress count.
#[test]
fn parse_count_extend_negative_clamps_to_one() {
    assert_eq!(
        parse_count_extend(&[SteelVal::IntV(-7)]).unwrap(),
        (Some(1), false)
    );
    assert_eq!(
        parse_count_extend(&[SteelVal::IntV(-1), SteelVal::BoolV(false)]).unwrap(),
        (Some(1), false)
    );
}

/// Zero is the Scheme spelling of "no count typed" — decodes to `None`,
/// not `Some(1)` (a bare keypress and an explicit count of 1 are different
/// dispatch origins even though both apply a command once).
#[test]
fn parse_count_extend_zero_means_no_count() {
    assert_eq!(
        parse_count_extend(&[SteelVal::IntV(0)]).unwrap(),
        (None, false)
    );
    assert_eq!(
        parse_count_extend(&[SteelVal::IntV(0), SteelVal::BoolV(true)]).unwrap(),
        (None, true)
    );
}

#[test]
fn parse_count_extend_string_arg_is_err() {
    let bad = &[SteelVal::StringV("garbage".into())];
    assert!(parse_count_extend(bad).is_err());
}

#[test]
fn parse_count_extend_extra_arg_is_err() {
    let bad = &[SteelVal::IntV(1), SteelVal::BoolV(false), SteelVal::IntV(0)];
    assert!(parse_count_extend(bad).is_err());
}

#[test]
fn command_plugin_unknown_returns_hume() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = command_plugin(&mut ctx, "move-right".to_string()).unwrap();
    assert_eq!(result, SteelVal::StringV("hume".into()));
}

#[test]
fn command_plugin_known_returns_owner() {
    let mut h = SteelCtxTestHarness::new();
    h.registries
        .cmd_owners
        .insert("my-cmd".to_string(), "core:plum".to_string());
    let mut ctx = h.ctx();
    let result = command_plugin(&mut ctx, "my-cmd".to_string()).unwrap();
    assert_eq!(result, SteelVal::StringV("core:plum".into()));
}

// ── define-command! validation ────────────────────────────────────────────

#[test]
fn define_command_name_with_double_quote_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = define_command(
        &mut ctx,
        "bad\"name".to_string(),
        "doc".to_string(),
        SteelVal::BoolV(false), // type check comes after name check
        false,
        false,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("must not contain"),
        "expected name rejection, got: {err}"
    );
}

#[test]
fn define_command_name_with_backslash_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = define_command(
        &mut ctx,
        "bad\\name".to_string(),
        "doc".to_string(),
        SteelVal::BoolV(false),
        false,
        false,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("must not contain"),
        "expected name rejection, got: {err}"
    );
}

/// When the host rejects the registration, `command_table` and `cmd_owners`
/// must stay clean — the host call runs *before* the table inserts.
///
/// Fail oracle: move the inserts back above `host.register_command` → the
/// entries linger after the Err and both cleanliness asserts fire.  A stale
/// entry would later make the plugin-failure rollback unregister a command
/// the plugin never actually owned.
#[test]
fn define_command_host_rejection_leaves_tables_clean() {
    fn dummy_proc(_args: &[SteelVal]) -> SteelResult {
        Ok(SteelVal::Void)
    }
    let mut h = SteelCtxTestHarness::new();
    let mut host = crate::null_host::FailingRegisterHost::default();
    {
        let mut ctx = h.ctx_init_with_host(&mut host);
        let err = define_command(
            &mut ctx,
            "rejected-cmd".to_string(),
            "doc".to_string(),
            SteelVal::FuncV(dummy_proc),
            false,
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("rejected by the command registry"),
            "error must come from the host; got: {err}"
        );
    }
    assert!(
        !h.registries.command_table.contains_key("rejected-cmd"),
        "command_table must not record a command the host rejected"
    );
    assert!(
        !h.registries.cmd_owners.contains_key("rejected-cmd"),
        "cmd_owners must not record a command the host rejected"
    );
}

#[test]
fn define_command_dup_names_error_names_existing_owner() {
    let mut h = SteelCtxTestHarness::new();
    // Simulate a command already fully defined by core:plum.
    // Both command_table (actually defined) and cmd_owners (attribution)
    // must be set — cmd_owners alone is pre-seeded by declare_plugin for
    // activation command ownership, so the guard checks command_table.
    h.registries
        .command_table
        .insert("my-cmd".to_string(), SteelVal::BoolV(false));
    h.registries
        .cmd_owners
        .insert("my-cmd".to_string(), "core:plum".to_string());
    let mut ctx = h.ctx_init();
    let err = define_command(
        &mut ctx,
        "my-cmd".to_string(),
        "doc".to_string(),
        SteelVal::BoolV(false),
        false,
        false,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("core:plum"),
        "error must name the existing owner; got: {msg}"
    );
    assert!(
        msg.contains("my-cmd"),
        "error must name the command; got: {msg}"
    );
}
