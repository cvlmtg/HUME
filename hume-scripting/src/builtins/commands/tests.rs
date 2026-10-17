use super::*;
use crate::test_support::SteelCtxTestHarness;

fn make_list(vals: Vec<SteelVal>) -> SteelVal {
    SteelVal::ListV(vals.into_iter().collect())
}

fn plum_entry() -> crate::attribution::EntryId {
    crate::attribution::EntryId::main(crate::attribution::PluginId::Core("plum".to_string()))
}

/// A host holding one command, `my-cmd`, defined as `proc`.
fn plum_defined(proc: CommandProc) -> crate::null_host::CommandStoreHost {
    let mut host = crate::null_host::CommandStoreHost::default();
    host.define("my-cmd", proc);
    host
}

/// NullHost returns `Ok(false)` for `command_is_native` (no registry) →
/// "not a native command" path → error logged, `#f` returned.
#[test]
fn call_bang_unknown_command_logs_error() {
    let mut h = SteelCtxTestHarness::new();
    let result = {
        let mut ctx = h.ctx_init();
        call_command_primitive(
            &mut ctx,
            "plum/ensure-grammars".to_string(),
            make_list(vec![]),
        )
        .unwrap()
    };
    assert_eq!(
        result,
        SteelVal::BoolV(false),
        "a miss must return #f, not #void: a Scheme guard like \
         `(when (call! …) …)` must treat it as a refusal, not a truthy no-op"
    );
    assert!(
        h.pending_messages
            .iter()
            .any(|(level, msg)| *level == LogLevel::Error && msg.contains("plum/ensure-grammars")),
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

// ── parse_native_args direct tests ───────────────────────────────────────

#[test]
fn parse_native_args_empty_gives_defaults() {
    assert_eq!(parse_native_args(&[]).unwrap(), (Some(1), false, None));
}

#[test]
fn parse_native_args_count_only() {
    assert_eq!(
        parse_native_args(&[SteelVal::IntV(5)]).unwrap(),
        (Some(5), false, None)
    );
}

#[test]
fn parse_native_args_count_and_extend() {
    assert_eq!(
        parse_native_args(&[SteelVal::IntV(3), SteelVal::BoolV(true)]).unwrap(),
        (Some(3), true, None)
    );
}

/// Negative counts clamp to `Some(1)`, same as a native keypress count.
#[test]
fn parse_native_args_negative_clamps_to_one() {
    assert_eq!(
        parse_native_args(&[SteelVal::IntV(-7)]).unwrap(),
        (Some(1), false, None)
    );
    assert_eq!(
        parse_native_args(&[SteelVal::IntV(-1), SteelVal::BoolV(false)]).unwrap(),
        (Some(1), false, None)
    );
}

/// Zero is the Scheme spelling of "no count typed" and decodes to `None`,
/// not `Some(1)` (a bare keypress and an explicit count of 1 are different
/// dispatch origins even though both apply a command once).
#[test]
fn parse_native_args_zero_means_no_count() {
    assert_eq!(
        parse_native_args(&[SteelVal::IntV(0)]).unwrap(),
        (None, false, None)
    );
    assert_eq!(
        parse_native_args(&[SteelVal::IntV(0), SteelVal::BoolV(true)]).unwrap(),
        (None, true, None)
    );
}

/// A script has no digit-by-digit accumulator to overflow, but can still
/// pass an arbitrary `isize` count straight through, so it must clamp to the same
/// ceiling the keyboard accumulator enforces, or a command that loops the
/// count with no fixed-point exit (e.g. macro replay) can be made to hang.
#[test]
fn parse_native_args_caps_at_max_count() {
    assert_eq!(
        parse_native_args(&[SteelVal::IntV(isize::MAX)]).unwrap(),
        (Some(MAX_COUNT), false, None)
    );
}

#[test]
fn parse_native_args_string_alone_is_text_arg() {
    assert_eq!(
        parse_native_args(&[SteelVal::StringV("x".into())]).unwrap(),
        (Some(1), false, Some(CommandArg::Text("x".to_string())))
    );
}

#[test]
fn parse_native_args_char_alone_is_char_arg() {
    assert_eq!(
        parse_native_args(&[SteelVal::CharV('x')]).unwrap(),
        (Some(1), false, Some(CommandArg::Char('x')))
    );
}

#[test]
fn parse_native_args_count_then_arg() {
    assert_eq!(
        parse_native_args(&[SteelVal::IntV(3), SteelVal::StringV("ab".into())]).unwrap(),
        (Some(3), false, Some(CommandArg::Text("ab".to_string())))
    );
}

#[test]
fn parse_native_args_count_extend_then_arg() {
    assert_eq!(
        parse_native_args(&[
            SteelVal::IntV(3),
            SteelVal::BoolV(true),
            SteelVal::CharV('y')
        ])
        .unwrap(),
        (Some(3), true, Some(CommandArg::Char('y')))
    );
}

#[test]
fn parse_native_args_two_trailing_args_is_err() {
    let bad = &[SteelVal::StringV("a".into()), SteelVal::StringV("b".into())];
    assert!(parse_native_args(bad).is_err());
}

#[test]
fn parse_native_args_non_text_arg_slot_is_err() {
    let int_slot = &[SteelVal::IntV(1), SteelVal::BoolV(false), SteelVal::IntV(0)];
    assert!(parse_native_args(int_slot).is_err());
    let bool_slot = &[
        SteelVal::IntV(1),
        SteelVal::BoolV(false),
        SteelVal::BoolV(true),
    ];
    assert!(parse_native_args(bool_slot).is_err());
    let list_slot = &[SteelVal::ListV(
        vec![SteelVal::CharV('x')].into_iter().collect(),
    )];
    assert!(parse_native_args(list_slot).is_err());
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
    let mut host = plum_defined(CommandProc::Mappable(MappableProc::new(
        SteelVal::BoolV(false),
        Owner::Plugin(plum_entry()),
    )));
    let mut ctx = h.ctx_init_with_host(&mut host);
    let result = command_plugin(&mut ctx, "my-cmd".to_string()).unwrap();
    assert_eq!(result, SteelVal::StringV("core:plum".into()));
}

// ── define-typed-command! validation ──────────────────────────────────────
//
// Mirrors "── define-command! validation ──" below.

#[test]
fn define_typed_command_name_with_double_quote_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = define_typed_command(
        &mut ctx,
        "bad\"name".to_string(),
        "doc".to_string(),
        SteelVal::BoolV(false), // type check comes after name check
        false,
        SteelVal::BoolV(false),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("must not contain"),
        "expected name rejection, got: {err}"
    );
}

#[test]
fn define_typed_command_name_with_backslash_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = define_typed_command(
        &mut ctx,
        "bad\\name".to_string(),
        "doc".to_string(),
        SteelVal::BoolV(false),
        false,
        SteelVal::BoolV(false),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("must not contain"),
        "expected name rejection, got: {err}"
    );
}

/// A name already defined in another case is the same command.
#[test]
fn define_typed_command_name_defined_in_another_case_errors() {
    fn dummy_proc(_args: &[SteelVal]) -> SteelResult {
        Ok(SteelVal::Void)
    }
    let mut host = plum_defined(CommandProc::Mappable(MappableProc::for_test()));
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init_with_host(&mut host);
    let err = define_typed_command(
        &mut ctx,
        "MY-CMD".to_string(),
        "doc".to_string(),
        SteelVal::FuncV(dummy_proc),
        false,
        SteelVal::BoolV(false),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("already defined"),
        "expected a re-definition error, got: {err}"
    );
}

/// A registry rejection surfaces as the define's own error.
#[test]
fn define_typed_command_propagates_host_rejection() {
    fn dummy_proc(_args: &[SteelVal]) -> SteelResult {
        Ok(SteelVal::Void)
    }
    let mut h = SteelCtxTestHarness::new();
    let mut host = crate::null_host::NullHost;
    let mut ctx = h.ctx_init_with_host(&mut host);
    let err = define_typed_command(
        &mut ctx,
        "rejected-cmd".to_string(),
        "doc".to_string(),
        SteelVal::FuncV(dummy_proc),
        false,
        SteelVal::BoolV(false),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("no command registry"),
        "error must come from the host; got: {err}"
    );
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

/// A registry rejection surfaces as the define's own error.
#[test]
fn define_command_propagates_host_rejection() {
    fn dummy_proc(_args: &[SteelVal]) -> SteelResult {
        Ok(SteelVal::Void)
    }
    let mut h = SteelCtxTestHarness::new();
    let mut host = crate::null_host::NullHost;
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
        err.to_string().contains("no command registry"),
        "error must come from the host; got: {err}"
    );
}

#[test]
fn define_command_dup_names_error_names_existing_owner() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = plum_defined(CommandProc::Mappable(MappableProc::new(
        SteelVal::BoolV(false),
        Owner::Plugin(plum_entry()),
    )));
    let mut ctx = h.ctx_init_with_host(&mut host);
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

/// Same collision, but both sides typed: `check_definable`'s re-definition
/// guard checks typed procs too, so a second `define-typed-command!` for
/// the same name names the owner here. The editor-side host's generic
/// "conflicts with existing command" error would not.
#[test]
fn define_typed_command_dup_names_error_names_existing_owner() {
    fn dummy_proc(_args: &[SteelVal]) -> SteelResult {
        Ok(SteelVal::Void)
    }
    let mut h = SteelCtxTestHarness::new();
    let mut host = plum_defined(CommandProc::Typed(TypedProc::new(
        SteelVal::BoolV(false),
        Owner::Plugin(plum_entry()),
    )));
    let mut ctx = h.ctx_init_with_host(&mut host);
    // A real callable proc: check_definable must reject before ever reaching
    // `callable_arg`'s type check, or this fails on the wrong error entirely.
    let err = define_typed_command(
        &mut ctx,
        "my-cmd".to_string(),
        "doc".to_string(),
        SteelVal::FuncV(dummy_proc),
        false,
        SteelVal::BoolV(false),
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
