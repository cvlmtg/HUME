use super::*;
use crate::null_host::RecordingInlineOutputHost;
use crate::test_support::SteelCtxTestHarness;
use tempfile::TempDir;

/// A missing `src` (and thus a failing `tree-sitter build`) logs a
/// Warning and returns void during init — a broken grammar must never
/// abort the editor on startup.
#[test]
fn compile_grammar_warns_instead_of_erroring_in_init_mode() {
    let tmp = TempDir::new().unwrap();
    let src = tmp
        .path()
        .join("does-not-exist")
        .to_string_lossy()
        .to_string();
    let out = tmp.path().join("out.dylib").to_string_lossy().to_string();

    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Init;
    let result = compile_grammar(&mut ctx, src, out);
    assert!(result.is_ok(), "init mode must not raise: {result:?}");
}

/// The same failure in command mode raises instead of silently warning.
#[test]
fn compile_grammar_raises_in_command_mode() {
    let tmp = TempDir::new().unwrap();
    let src = tmp
        .path()
        .join("does-not-exist")
        .to_string_lossy()
        .to_string();
    let out = tmp.path().join("out.dylib").to_string_lossy().to_string();

    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Runtime;
    assert!(compile_grammar(&mut ctx, src, out).is_err());
}

/// `tree-sitter build` inherits stdio — the bracket must open before the
/// spawn attempt in command mode, even when the build itself then fails.
#[test]
fn compile_grammar_calls_ensure_before_build_in_command_mode() {
    let tmp = TempDir::new().unwrap();
    let src = tmp
        .path()
        .join("does-not-exist")
        .to_string_lossy()
        .to_string();
    let out = tmp.path().join("out.dylib").to_string_lossy().to_string();

    let mut host = RecordingInlineOutputHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    ctx.session = crate::context::EvalSession::Runtime;
    let _ = compile_grammar(&mut ctx, src, out);
    drop(ctx);
    assert_eq!(host.ensure_calls, 1);
}

/// Init-time compiles run pre-terminal — the bracket must never open.
#[test]
fn compile_grammar_does_not_call_ensure_in_init_mode() {
    let tmp = TempDir::new().unwrap();
    let src = tmp
        .path()
        .join("does-not-exist")
        .to_string_lossy()
        .to_string();
    let out = tmp.path().join("out.dylib").to_string_lossy().to_string();

    let mut host = RecordingInlineOutputHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    ctx.session = crate::context::EvalSession::Init;
    let _ = compile_grammar(&mut ctx, src, out);
    drop(ctx);
    assert_eq!(host.ensure_calls, 0);
}
