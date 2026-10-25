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
        relative_to(
            &AbsolutePath::rooted(r"\\?\C:\a\sub\f.rs"),
            &AbsolutePath::rooted(r"C:\a")
        ),
        expected
    );
    assert_eq!(
        relative_to(
            &AbsolutePath::rooted(r"C:\a\sub\f.rs"),
            &AbsolutePath::rooted(r"\\?\C:\a")
        ),
        expected
    );
}

// ── completion_entries ────────────────────────────────────────────────────

#[test]
fn completion_entries_matches_the_prefix_in_any_case() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.txt"), b"").unwrap();
    assert_eq!(completion_entries(dir.path(), "PRO", false), ["probe.txt"]);
}

// ── absolute ──────────────────────────────────────────────────────────────

#[test]
fn absolute_rejects_a_drive_relative_path() {
    let err = absolute(Path::new("D:b.txt"), Path::new(r"C:\home\u")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

/// Windows resolves `..` by name, without reading the disk, so a `..` after
/// a missing directory still names a path.
#[test]
fn absolute_resolves_dot_dot_by_name() {
    let got = absolute(Path::new(r"missing\..\f.txt"), Path::new(r"C:\home\u")).unwrap();
    assert_eq!(got, Path::new(r"C:\home\u\f.txt"));
}
