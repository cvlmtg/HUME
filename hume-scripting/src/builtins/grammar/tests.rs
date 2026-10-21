use super::*;
use crate::builtins::dirs::ScriptDirs;
use crate::null_host::RecordingInlineOutputHost;
use crate::test_support::SteelCtxTestHarness;
use hume_platform::dirs::{Dirs, GrammarBuild};
use std::path::PathBuf;
use tempfile::TempDir;

/// A harness whose grammar compiler is a missing file in `tmp`, so every
/// compile fails to spawn, plus a missing `src` and an `out` inside `tmp`.
fn failing_compile(tmp: &TempDir) -> (SteelCtxTestHarness, String, String) {
    let mut h = SteelCtxTestHarness::new();
    h.dirs = ScriptDirs::new(&Dirs {
        grammar_build: GrammarBuild::Found(tmp.path().join("hume-grammar-build")),
        ..Dirs::none()
    });
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
    let mut ctx = h.ctx_init();
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
    assert!(compile_grammar(&mut ctx, src, out).is_err());
    drop(ctx);
    assert_eq!(host.ensure_calls, 1);
}

/// The error raised when no grammar compiler was found at startup.
fn missing_compiler_error(grammar_build: GrammarBuild) -> String {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().to_string_lossy().to_string();
    let out = tmp.path().join("out.dylib").to_string_lossy().to_string();

    let mut h = SteelCtxTestHarness::new();
    h.dirs = ScriptDirs::new(&Dirs {
        grammar_build,
        ..Dirs::none()
    });
    let mut ctx = h.ctx();
    compile_grammar(&mut ctx, src, out).unwrap_err().to_string()
}

/// A user who finds no grammar compiler is told every path HUME looked at.
#[test]
fn compile_grammar_without_a_grammar_compiler_names_where_it_looked() {
    let searched = vec![
        PathBuf::from("/opt/hume/libexec/hume/hume-grammar-build"),
        PathBuf::from("/opt/hume/bin/hume-grammar-build"),
    ];
    let err = missing_compiler_error(GrammarBuild::Missing {
        searched: searched.clone(),
    });
    for path in &searched {
        assert!(
            err.contains(&path.display().to_string()),
            "{path:?} missing from: {err}"
        );
    }
}

#[test]
fn compile_grammar_without_an_executable_path_says_so() {
    let err = missing_compiler_error(GrammarBuild::ExeUnknown);
    assert!(
        err.contains("location of the hume executable is unknown"),
        "unexpected error: {err}"
    );
}

/// Init-time compiles run pre-terminal, so the bracket must never open.
#[test]
fn compile_grammar_does_not_call_ensure_in_init_mode() {
    let tmp = TempDir::new().unwrap();
    let (mut h, src, out) = failing_compile(&tmp);
    let mut host = RecordingInlineOutputHost::default();
    let mut ctx = h.ctx_init_with_host(&mut host);
    assert!(compile_grammar(&mut ctx, src, out).is_ok());
    drop(ctx);
    assert_eq!(host.ensure_calls, 0);
}
