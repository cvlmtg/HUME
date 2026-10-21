use super::*;
use crate::builtins::dirs::ScriptDirs;
use crate::null_host::RecordingInlineOutputHost;
use crate::test_support::SteelCtxTestHarness;
use tempfile::TempDir;

/// A harness whose grammar compiler is a missing file in `tmp`, so every
/// compile fails to spawn, plus a missing `src` and an `out` inside `tmp`.
fn failing_compile(tmp: &TempDir) -> (SteelCtxTestHarness, String, String) {
    let mut h = SteelCtxTestHarness::new();
    h.dirs = ScriptDirs::new(None, None, Some(tmp.path().join("hume-grammar-build")));
    let src = tmp
        .path()
        .join("does-not-exist")
        .to_string_lossy()
        .to_string();
    let out = tmp.path().join("out.dylib").to_string_lossy().to_string();
    (h, src, out)
}

/// A failing compile logs a Warning and returns void during init: a broken
/// grammar must never abort the editor on startup.
#[test]
fn compile_grammar_warns_instead_of_erroring_in_init_mode() {
    let tmp = TempDir::new().unwrap();
    let (mut h, src, out) = failing_compile(&tmp);
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Init;
    let result = compile_grammar(&mut ctx, src, out);
    assert!(result.is_ok(), "init mode must not raise: {result:?}");
    drop(ctx);
    assert!(
        h.pending_messages
            .iter()
            .any(|(level, msg)| *level == LogLevel::Warning && msg.contains("cannot run")),
        "expected a spawn-failure warning: {:?}",
        h.pending_messages
    );
}

/// The same failure in command mode raises instead of silently warning.
#[test]
fn compile_grammar_raises_in_command_mode() {
    let tmp = TempDir::new().unwrap();
    let (mut h, src, out) = failing_compile(&tmp);
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Runtime;
    let err = compile_grammar(&mut ctx, src, out).unwrap_err();
    assert!(
        err.to_string().contains("cannot run"),
        "unexpected error: {err}"
    );
}

/// The compiler inherits stdio, so the bracket must open before the spawn
/// attempt in command mode, even when the build itself then fails.
#[test]
fn compile_grammar_calls_ensure_before_build_in_command_mode() {
    let tmp = TempDir::new().unwrap();
    let (mut h, src, out) = failing_compile(&tmp);
    let mut host = RecordingInlineOutputHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    ctx.session = crate::context::EvalSession::Runtime;
    assert!(compile_grammar(&mut ctx, src, out).is_err());
    drop(ctx);
    assert_eq!(host.ensure_calls, 1);
}

/// With no grammar compiler found at startup there is nothing to run.
#[test]
fn compile_grammar_without_a_grammar_compiler_raises_naming_it() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().to_string_lossy().to_string();
    let out = tmp.path().join("out.dylib").to_string_lossy().to_string();

    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Runtime;
    let err = compile_grammar(&mut ctx, src, out).unwrap_err();
    assert!(
        err.to_string()
            .contains("hume-grammar-build is not installed beside the hume executable"),
        "unexpected error: {err}"
    );
}

/// Init-time compiles run pre-terminal, so the bracket must never open.
#[test]
fn compile_grammar_does_not_call_ensure_in_init_mode() {
    let tmp = TempDir::new().unwrap();
    let (mut h, src, out) = failing_compile(&tmp);
    let mut host = RecordingInlineOutputHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    ctx.session = crate::context::EvalSession::Init;
    assert!(compile_grammar(&mut ctx, src, out).is_ok());
    drop(ctx);
    assert_eq!(host.ensure_calls, 0);
}
