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

// ── path->display ─────────────────────────────────────────────────────────

fn call_path_to_display(path: &str) -> String {
    let args = vec![SteelVal::StringV(path.into())];
    match path_to_display(&args).unwrap() {
        SteelVal::StringV(s) => s.to_string(),
        other => panic!("expected string, got {other:?}"),
    }
}

/// A path with no home prefix and no Windows verbatim prefix must come back
/// byte-identical — an oracle independent of `display_form`'s own logic,
/// unlike comparing against `display_form`'s output directly.
#[test]
fn path_to_display_leaves_unrelated_path_unchanged() {
    let input = "/some/absolute/path/file.rs";
    assert_eq!(call_path_to_display(input), input);
}

/// `~`-collapse: a path under `$HOME` must come back with the home prefix
/// replaced by `~`, built here from the raw separator rather than by calling
/// `display_form`/`shorten_home` — the two behaviours this builtin exists for
/// (this one and UNC-strip below) must each have a hand-computed expectation.
#[test]
fn path_to_display_collapses_home_prefix() {
    let Some(home) = hume_platform::dirs::home_dir() else {
        return; // no $HOME in this environment — nothing to collapse against
    };
    let input = home.join("dev").join("hume").join("x.rs");
    let sep = std::path::MAIN_SEPARATOR;
    let expected = format!("~{sep}dev{sep}hume{sep}x.rs");
    assert_eq!(call_path_to_display(input.to_str().unwrap()), expected);
}

/// UNC-strip: a Windows verbatim-prefixed path must have `\\?\` removed,
/// leaving a plain drive-letter path.
#[cfg(windows)]
#[test]
fn path_to_display_strips_windows_verbatim_prefix() {
    assert_eq!(
        call_path_to_display(r"\\?\C:\Users\x\file.rs"),
        r"C:\Users\x\file.rs"
    );
}

#[test]
fn path_to_display_no_args_errors() {
    assert!(path_to_display(&[]).is_err());
}

#[test]
fn path_to_display_type_error() {
    let args = vec![SteelVal::IntV(42)];
    assert!(path_to_display(&args).is_err());
}

// ── path-separator (steel-core builtin, not registered by HUME) ──────────

/// `(path-separator)` is not a HUME builtin — it comes from steel-core's
/// `steel/meta` module, already a bare global in `Engine::new()`. Prove it
/// resolves *through a loaded plugin*, not just at the top level: a
/// `register_value` of a non-function value is known to silently stub out
/// inside `load-plugin`'d code (see memory
/// `reference_steel_load_plugin_bare_globals`), and this name must not
/// regress to a HUME-registered shadow that could reintroduce that trap.
///
/// Fail oracle: reintroducing `fs::path_separator` registered as a bare
/// value (not a niladic function) would make this test hang or error
/// instead of logging the separator.
#[test]
fn path_separator_resolves_inside_loaded_plugin() {
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("probe");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(log! 'info (path-separator))"#,
    )
    .unwrap();

    let mut host = crate::ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    let mut null_host = crate::null_host::NullHost;
    host.eval_source(r#"(load-plugin "user/probe")"#, &mut null_host)
        .expect("(path-separator) must be callable from inside a loaded plugin");

    let msgs = host.take_pending_messages();
    assert!(
        msgs.iter()
            .any(|(_, msg)| msg == &std::path::MAIN_SEPARATOR.to_string()),
        "expected a log message equal to {:?}, got: {:?}",
        std::path::MAIN_SEPARATOR.to_string(),
        msgs
    );
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
    let expected = hume_platform::path::strip_unc_prefix(std::fs::canonicalize(&data_dir).unwrap());
    let msgs = host.take_pending_messages();
    assert!(
        msgs.iter()
            .any(|(_, msg)| msg == &expected.to_string_lossy()),
        "expected a log message equal to {:?}, got: {:?}",
        expected,
        msgs
    );
}
