use super::*;
use crate::builtins::dirs::ScriptDirs;
use crate::test_support::SteelCtxTestHarness;
use tempfile::TempDir;

fn harness_with_data_dir(tmp: &TempDir) -> SteelCtxTestHarness {
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut h = SteelCtxTestHarness::new();
    h.dirs = ScriptDirs::new(Some(data_dir), None);
    h
}

// ── path-join ────────────────────────────────────────────────────────────

#[test]
fn path_join_two_segments() {
    let args = vec![
        SteelVal::StringV("foo".into()),
        SteelVal::StringV("bar".into()),
    ];
    let result = path_join(&args).unwrap();
    let s = match result {
        SteelVal::StringV(s) => s.to_string(),
        other => panic!("expected string, got {other:?}"),
    };
    // The joined path must contain both components separated by the OS separator.
    let expected = std::path::PathBuf::from("foo").join("bar");
    assert_eq!(s, expected.to_string_lossy().as_ref());
}

#[test]
fn path_join_single_segment() {
    let args = vec![SteelVal::StringV("only".into())];
    let result = path_join(&args).unwrap();
    assert!(matches!(result, SteelVal::StringV(s) if s.as_str() == "only"));
}

#[test]
fn path_join_no_args_errors() {
    assert!(path_join(&[]).is_err());
}

#[test]
fn path_join_type_error() {
    let args = vec![SteelVal::IntV(42)];
    assert!(path_join(&args).is_err());
}

// ── data-dir display (no UNC prefix) ─────────────────────────────────────

/// On all platforms `(data-dir)` must return a string that does not begin
/// with the Windows extended-length prefix `\\?\`.  On Unix this prefix
/// never appears, so the test is platform-neutral.
#[test]
fn data_dir_no_unc_prefix() {
    let tmp = TempDir::new().unwrap();
    let mut h = harness_with_data_dir(&tmp);
    let mut ctx = h.ctx();

    let result = data_dir(&mut ctx).unwrap();
    let s = match result {
        SteelVal::StringV(s) => s.to_string(),
        other => panic!("expected string, got {other:?}"),
    };
    assert!(
        !s.starts_with(r"\\?\"),
        "data-dir must not return an extended-length UNC path, got: {s}"
    );
}

/// End-to-end through the real registration table (`builtins::mod`'s
/// `builtins!` table registers `data-dir` as a ctx-injected `open`
/// builtin) — proves `(data-dir)` resolves to `ctx.dirs` through that path.
/// `eval_source` only reports success/failure, not a return value, so
/// the result is round-tripped through `log!` and read back from the
/// message log.
#[test]
fn data_dir_resolves_through_real_registration() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut host = crate::ScriptingHost::new();
    host.set_data_dir(data_dir.clone());
    let mut null_host = crate::null_host::NullHost;
    host.eval_source(r#"(log! 'info (data-dir))"#, &mut null_host)
        .expect("(data-dir) must evaluate through the real registration table");

    // macOS TempDir paths are under /var, which is itself a symlink to
    // /private/var — canonicalize the *expected* side so the comparison
    // isn't platform-dependent (see memory feedback_macos_tempfile_canonicalize).
    // On Windows canonicalize yields a `\\?\`-prefixed path, but
    // `(data-dir)` returns the display form — strip the prefix to match.
    let expected =
        hume_platform::path::strip_unc_prefix(std::fs::canonicalize(&data_dir).unwrap());
    let msgs = host.take_pending_messages();
    assert!(
        msgs.iter()
            .any(|(_, msg)| msg == &expected.to_string_lossy()),
        "expected a log message equal to {:?}, got: {:?}",
        expected,
        msgs
    );
}
