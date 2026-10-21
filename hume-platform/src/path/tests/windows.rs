//! Windows-only tests, gated once at the `mod windows;`
//! declaration in the parent.

use super::*;

// ── Windows env-var expansion ─────────────────────────────────────────────

#[test]
fn expand_percent_var() {
    let result = expand_with(
        r"%USERPROFILE%\foo",
        |k| {
            if k == "USERPROFILE" {
                Ok(r"C:\Users\Alice".into())
            } else {
                Err(std::env::VarError::NotPresent)
            }
        },
        no_home,
    );
    assert_eq!(result, r"C:\Users\Alice\foo");
}

#[test]
fn expand_unknown_percent_var_stays_literal() {
    let result = expand_with(r"%NONEXISTENT%\foo", no_env, no_home);
    assert_eq!(result, r"%NONEXISTENT%\foo");
}

#[test]
fn expand_unclosed_percent_stays_literal() {
    let result = expand_with(r"%UNCLOSED\foo", no_env, no_home);
    assert_eq!(result, r"%UNCLOSED\foo");
}

#[test]
fn expand_consecutive_percent_vars() {
    let result = expand_with(
        "%A%%B%",
        |k| match k {
            "A" => Ok("x".into()),
            "B" => Ok("y".into()),
            _ => Err(std::env::VarError::NotPresent),
        },
        no_home,
    );
    assert_eq!(result, "xy");
}

#[test]
fn expand_tilde_and_percent_var() {
    let result = expand_with(
        r"~\%DIR%\file",
        |k| {
            if k == "DIR" {
                Ok("docs".into())
            } else {
                Err(std::env::VarError::NotPresent)
            }
        },
        home(r"C:\Users\Alice"),
    );
    assert_eq!(result, r"C:\Users\Alice\docs\file");
}

// ── strip_unc_prefix ─────────────────────────────────────────────────────

#[test]
fn strip_unc_prefix_removes_verbatim_drive_prefix() {
    let got = strip_unc_prefix(PathBuf::from(r"\\?\C:\Users\x"));
    assert_eq!(got, PathBuf::from(r"C:\Users\x"));
}

#[test]
fn strip_unc_prefix_leaves_verbatim_unc_unchanged() {
    let got = strip_unc_prefix(PathBuf::from(r"\\?\UNC\server\share"));
    assert_eq!(got, PathBuf::from(r"\\?\UNC\server\share"));
}

#[test]
fn strip_unc_prefix_plain_path_unchanged() {
    let got = strip_unc_prefix(PathBuf::from(r"C:\Users\x"));
    assert_eq!(got, PathBuf::from(r"C:\Users\x"));
}

// ── display_form_with ─────────────────────────────────────────────────────

#[test]
fn display_form_with_strips_unc_then_shortens_home() {
    let got = display_form_with(Path::new(r"\\?\C:\Users\Alice\dev\hume"), || {
        Some(PathBuf::from(r"C:\Users\Alice"))
    });
    assert_eq!(got, r"~\dev\hume");
}

#[test]
fn display_form_with_verbatim_unc_passthrough() {
    let got = display_form_with(Path::new(r"\\?\UNC\server\share"), || {
        Some(PathBuf::from(r"C:\Users\Alice"))
    });
    assert_eq!(got, r"\\?\UNC\server\share");
}

// ── relative_to ───────────────────────────────────────────────────────────

#[test]
fn relative_to_ignores_a_verbatim_prefix_on_either_side() {
    let expected = Some(PathBuf::from(r"sub\f.rs"));
    assert_eq!(
        relative_to(Path::new(r"\\?\C:\a\sub\f.rs"), Path::new(r"C:\a")),
        expected
    );
    assert_eq!(
        relative_to(Path::new(r"C:\a\sub\f.rs"), Path::new(r"\\?\C:\a")),
        expected
    );
}

// ── executables_in ────────────────────────────────────────────────────────

#[test]
fn executables_in_lists_an_exe_by_its_stem_and_nothing_else() {
    let bin = tempfile::tempdir().unwrap();
    for name in [
        "probe.exe",
        "probe-upper.EXE",
        "probe-script.cmd",
        "probe-batch.bat",
        "probe-notes.txt",
        "probe",
    ] {
        std::fs::write(bin.path().join(name), b"").unwrap();
    }
    assert_eq!(
        executables_in(&search_path([bin.path()]), "probe", &[".exe"]),
        ["probe", "probe-upper"]
    );
}

#[test]
fn executables_in_lists_each_given_extension_by_its_stem() {
    let bin = tempfile::tempdir().unwrap();
    for name in [
        "probe.exe",
        "probe-script.cmd",
        "probe-batch.BAT",
        "probe-shell.ps1",
    ] {
        std::fs::write(bin.path().join(name), b"").unwrap();
    }
    assert_eq!(
        executables_in(
            &search_path([bin.path()]),
            "probe",
            &[".EXE", ".bat", ".ps1"]
        ),
        ["probe", "probe-batch", "probe-shell"]
    );
}

#[test]
fn executables_in_ignores_case_and_keeps_the_first_spelling_on_the_path() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("Code.exe"), b"").unwrap();
    std::fs::write(second.path().join("CODE.exe"), b"").unwrap();
    std::fs::write(second.path().join("codex.exe"), b"").unwrap();
    assert_eq!(
        executables_in(
            &search_path([first.path(), second.path()]),
            "cod",
            &[".exe"]
        ),
        ["Code", "codex"]
    );
}

#[test]
fn executables_in_matches_the_prefix_against_the_stem() {
    let bin = tempfile::tempdir().unwrap();
    std::fs::write(bin.path().join("bash.exe"), b"").unwrap();
    assert_eq!(
        executables_in(&search_path([bin.path()]), "bash.e", &[".exe"]),
        Vec::<String>::new()
    );
}
