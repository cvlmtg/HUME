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

