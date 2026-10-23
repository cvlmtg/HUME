use super::*;
use crate::builtins::dirs::ScriptDirs;
use crate::test_support::SteelCtxTestHarness;
use hume_platform::dirs::Dirs;
use tempfile::TempDir;

fn harness_with_data_dir(tmp: &TempDir) -> SteelCtxTestHarness {
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut h = SteelCtxTestHarness::new();
    h.dirs = ScriptDirs::new(&Dirs {
        data: Some(data_dir),
        ..Dirs::none()
    });
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

// ── safe-path-segment? ───────────────────────────────────────────────────

fn is_safe(arg: SteelVal) -> bool {
    match safe_path_segment(&[arg]).unwrap() {
        SteelVal::BoolV(b) => b,
        other => panic!("expected bool, got {other:?}"),
    }
}

#[test]
fn safe_path_segment_rejects_unsafe_names() {
    for name in ["", ".", "..", "a/b", "a\\b", "c:evil", "a\"b", "a\0b"] {
        assert!(!is_safe(SteelVal::StringV(name.into())), "{name:?}");
    }
}

#[test]
fn safe_path_segment_accepts_ordinary_names() {
    for name in ["v1.2.3", "rust-analyzer", "markdown.inline"] {
        assert!(is_safe(SteelVal::StringV(name.into())), "{name:?}");
    }
}

#[test]
fn safe_path_segment_answers_false_for_a_non_string() {
    assert!(!is_safe(SteelVal::IntV(42)));
    assert!(!is_safe(SteelVal::BoolV(false)));
}

#[test]
fn safe_path_segment_wrong_arity_errors() {
    assert!(safe_path_segment(&[]).is_err());
    assert!(
        safe_path_segment(&[SteelVal::StringV("a".into()), SteelVal::StringV("b".into())]).is_err()
    );
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
/// byte-identical. Comparing against the input keeps `display_form`'s own
/// logic out of the expected value.
#[test]
fn path_to_display_leaves_unrelated_path_unchanged() {
    let input = "/some/absolute/path/file.rs";
    assert_eq!(call_path_to_display(input), input);
}

/// `~`-collapse: a path under `$HOME` must come back with the home prefix
/// replaced by `~`, built here from the raw separator rather than by calling
/// `display_form`/`shorten_home`: the two behaviours this builtin exists for
/// (this one and UNC-strip below) must each have a hand-computed expectation.
#[test]
fn path_to_display_collapses_home_prefix() {
    let Some(home) = hume_platform::dirs::home_dir() else {
        return; // no $HOME in this environment, nothing to collapse against
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

/// `(path-separator)` is not a HUME builtin: it comes from steel-core's
/// `steel/meta` module, already a bare global in `Engine::new()`. Prove it
/// resolves *through a loaded plugin*, not just at the top level: a
/// `register_value` of a non-function value is known to silently stub out
/// inside `load-plugin!`'d code, and this name must not
/// regress to a HUME-registered shadow that could reintroduce that trap.
///
/// An `fs::path_separator` registered as a bare value instead of a niladic
/// function would make this test hang or error.
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

    let mut host = crate::ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    let mut null_host = crate::null_host::NullHost;
    host.eval_source(r#"(load-plugin! "user/probe")"#, &mut null_host)
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
/// builtin), proving `(data-dir)` resolves to `ctx.dirs` through that path.
/// `eval_source` only reports success/failure, not a return value, so
/// the result is round-tripped through `log!` and read back from the
/// message log.
#[test]
fn data_dir_resolves_through_real_registration() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut host = crate::ScriptingHost::new(&Dirs {
        data: Some(data_dir.clone()),
        ..Dirs::none()
    });
    let mut null_host = crate::null_host::NullHost;
    host.eval_source(r#"(log! 'info (data-dir))"#, &mut null_host)
        .expect("(data-dir) must evaluate through the real registration table");

    // macOS TempDir paths are under /var, which is itself a symlink to
    // /private/var, so canonicalize the *expected* side so the comparison
    // isn't platform-dependent.
    // On Windows canonicalize yields a `\\?\`-prefixed path, but
    // `(data-dir)` returns the display form, so strip the prefix to match.
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

// ── completion listings ──────────────────────────────────────────────────

/// `source`'s value, evaluated by a real `ScriptingHost` over `NullHost`.
fn eval_value(source: &str) -> SteelVal {
    let mut host = crate::ScriptingHost::new(&Dirs::none());
    host.eval_source_for_value(source, &mut crate::null_host::NullHost)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// The strings of a Steel list.
fn strings(list: SteelVal) -> Vec<String> {
    crate::builtins::args::list_to_strings(list, "test").unwrap()
}

#[test]
fn path_entries_lists_what_a_typed_path_completes_to() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a dir")).unwrap();
    std::fs::write(dir.path().join("b.txt"), b"").unwrap();
    std::fs::write(dir.path().join("other"), b"").unwrap();
    std::fs::write(dir.path().join(".hidden"), b"").unwrap();
    let listed = |prefix: &str| {
        strings(eval_value(&format!(
            "(path-entries {:?} {prefix:?})",
            dir.path().to_string_lossy()
        )))
    };
    assert_eq!(listed(""), ["a dir/", "b.txt", "other"]);
    assert_eq!(listed("b"), ["b.txt"]);
    assert_eq!(listed("."), [".hidden"]);
}

#[test]
fn path_entries_of_an_unreadable_directory_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    assert_eq!(
        strings(eval_value(&format!(
            "(path-entries {:?} \"\")",
            missing.to_string_lossy()
        ))),
        Vec::<String>::new()
    );
}

/// The error `path-programs` raises for `extensions` on a host with no
/// background-work capability.
fn path_programs_error(extensions: SteelVal) -> String {
    let mut h = SteelCtxTestHarness::new();
    path_programs(
        &mut h.ctx(),
        String::new(),
        extensions,
        crate::Rooted::new(SteelVal::Void),
    )
    .unwrap_err()
    .to_string()
}

#[test]
fn path_programs_reads_its_extensions_as_strings() {
    let err = path_programs_error(SteelVal::ListV(vec![SteelVal::IntV(1)].into()));
    assert!(!err.contains("not supported"), "{err}");
}

#[test]
fn path_programs_raises_without_background_work() {
    let err = path_programs_error(crate::builtins::args::string_list([".exe".to_owned()]));
    assert!(err.contains("path-programs: not supported"), "{err}");
}

#[test]
fn home_dir_is_the_platform_home_directory() {
    let expected = hume_platform::dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
    let got = match eval_value("(home-dir)") {
        SteelVal::StringV(s) => Some(s.to_string()),
        SteelVal::BoolV(false) => None,
        other => panic!("expected a string or #f, got {other:?}"),
    };
    assert_eq!(got, expected);
}

// ── replace-dir! ─────────────────────────────────────────────────────────

/// The names in `dir`, sorted.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("readable dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// A directory at `dir` holding one empty file named `file`.
fn dir_with(dir: &Path, file: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), "").unwrap();
}

fn call_replace_dir(src: &Path, dest: &Path) -> SteelResult {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    replace_dir(&mut ctx, src.to_path_buf(), dest.to_path_buf())
}

#[test]
fn replace_dir_puts_src_where_dest_was() {
    let tmp = TempDir::new().unwrap();
    let (src, parent) = (tmp.path().join("stage/repo"), tmp.path().join("plugins"));
    let dest = parent.join("repo");
    dir_with(&src, "new");
    dir_with(&dest, "old");

    call_replace_dir(&src, &dest).unwrap();

    assert_eq!(entries(&dest), ["new"]);
    assert_eq!(entries(&parent), ["repo"], "nothing is left beside dest");
    assert!(!src.exists());
}

#[test]
fn replace_dir_creates_a_missing_dest_and_its_parent() {
    let tmp = TempDir::new().unwrap();
    let (src, dest) = (tmp.path().join("stage/repo"), tmp.path().join("a/b/repo"));
    dir_with(&src, "new");

    call_replace_dir(&src, &dest).unwrap();

    assert_eq!(entries(&dest), ["new"]);
    assert!(!src.exists());
}

#[test]
fn replace_dir_keeps_dest_when_src_cannot_move() {
    let tmp = TempDir::new().unwrap();
    let (src, parent) = (tmp.path().join("stage/missing"), tmp.path().join("plugins"));
    let dest = parent.join("repo");
    dir_with(&dest, "old");

    call_replace_dir(&src, &dest).unwrap_err();

    assert_eq!(entries(&dest), ["old"]);
    assert_eq!(entries(&parent), ["repo"], "nothing is left beside dest");
}

/// A swap a crash interrupted leaves dest's backup behind; the next one
/// clears it.
#[test]
fn replace_dir_clears_a_backup_an_earlier_swap_left() {
    let tmp = TempDir::new().unwrap();
    let (src, parent) = (tmp.path().join("stage/repo"), tmp.path().join("plugins"));
    let dest = parent.join("repo");
    dir_with(&src, "new");
    dir_with(&dest, "old");
    dir_with(&parent.join("repo.replaced"), "older");

    call_replace_dir(&src, &dest).unwrap();

    assert_eq!(entries(&dest), ["new"]);
    assert_eq!(entries(&parent), ["repo"]);
}
