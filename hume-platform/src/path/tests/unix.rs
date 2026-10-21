//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;
use crate::process::test_support::write_program;

// ── Unix env-var expansion ────────────────────────────────────────────────

#[test]
fn expand_dollar_var() {
    let result = expand_with(
        "$HOME/foo",
        |k| {
            if k == "HOME" {
                Ok("/h".into())
            } else {
                Err(std::env::VarError::NotPresent)
            }
        },
        no_home,
    );
    assert_eq!(result, "/h/foo");
}

#[test]
fn expand_dollar_braced_var() {
    let result = expand_with(
        "${HOME}/foo",
        |k| {
            if k == "HOME" {
                Ok("/h".into())
            } else {
                Err(std::env::VarError::NotPresent)
            }
        },
        no_home,
    );
    assert_eq!(result, "/h/foo");
}

#[test]
fn expand_unknown_var_stays_literal() {
    let result = expand_with("$NONEXISTENT/foo", no_env, no_home);
    assert_eq!(result, "$NONEXISTENT/foo");
}

#[test]
fn expand_unclosed_brace_stays_literal() {
    let result = expand_with("${UNCLOSED/foo", no_env, no_home);
    // `${UNCLOSED` has no closing `}` before `/`, so it's emitted literally.
    assert_eq!(result, "${UNCLOSED/foo");
}

#[test]
fn expand_trailing_dollar_is_literal() {
    let result = expand_with("/foo$", no_env, no_home);
    assert_eq!(result, "/foo$");
}

#[test]
fn expand_dollar_digit_is_literal() {
    // `$1var`: a digit cannot start an identifier.
    let result = expand_with("$1var", no_env, no_home);
    assert_eq!(result, "$1var");
}

#[test]
fn expand_multiple_vars() {
    let result = expand_with(
        "$A/$B",
        |k| match k {
            "A" => Ok("x".into()),
            "B" => Ok("y".into()),
            _ => Err(std::env::VarError::NotPresent),
        },
        no_home,
    );
    assert_eq!(result, "x/y");
}

#[test]
fn expand_tilde_and_env_var() {
    let result = expand_with(
        "~/$DIR/file",
        |k| {
            if k == "DIR" {
                Ok("docs".into())
            } else {
                Err(std::env::VarError::NotPresent)
            }
        },
        home("/h"),
    );
    assert_eq!(result, "/h/docs/file");
}

// ── shorten_home_with ─────────────────────────────────────────────────────

#[test]
fn shorten_home_with_path_inside_home() {
    use std::path::Path;
    let got = shorten_home_with(Path::new("/home/user/dev/hume"), || {
        Some(PathBuf::from("/home/user"))
    });
    assert_eq!(got, "~/dev/hume");
}

#[test]
fn shorten_home_with_path_equal_to_home() {
    use std::path::Path;
    let got = shorten_home_with(Path::new("/home/user"), || {
        Some(PathBuf::from("/home/user"))
    });
    assert_eq!(got, "~");
}

#[test]
fn shorten_home_with_path_outside_home() {
    use std::path::Path;
    let got = shorten_home_with(Path::new("/tmp/foo"), || Some(PathBuf::from("/home/user")));
    assert_eq!(got, "/tmp/foo");
}

#[test]
fn shorten_home_with_no_home_returns_full_path() {
    use std::path::Path;
    let got = shorten_home_with(Path::new("/tmp/foo"), || None);
    assert_eq!(got, "/tmp/foo");
}

#[test]
fn shorten_home_with_path_prefix_not_a_dir_boundary() {
    use std::path::Path;
    // "/home/userx" must NOT match home="/home/user": strip_prefix is
    // component-aware and rejects partial component matches.
    let got = shorten_home_with(Path::new("/home/userx/foo"), || {
        Some(PathBuf::from("/home/user"))
    });
    assert_eq!(got, "/home/userx/foo");
}

#[test]
fn strip_unc_prefix_is_noop_on_non_windows() {
    let got = strip_unc_prefix(PathBuf::from("/tmp/foo"));
    assert_eq!(got, PathBuf::from("/tmp/foo"));
}

// ── display_form_with ────────────────────────────────────────────────────

#[test]
fn display_form_with_path_inside_home() {
    use std::path::Path;
    let got = display_form_with(Path::new("/home/user/dev/hume"), || {
        Some(PathBuf::from("/home/user"))
    });
    assert_eq!(got, "~/dev/hume");
}

#[test]
fn display_form_with_no_home_returns_full_path() {
    use std::path::Path;
    let got = display_form_with(Path::new("/tmp/foo"), || None);
    assert_eq!(got, "/tmp/foo");
}

// ── executables_in ────────────────────────────────────────────────────────

#[test]
fn executables_in_lists_files_with_an_execute_bit() {
    let bin = tempfile::tempdir().unwrap();
    write_program(bin.path(), "probe-owner", 0o744);
    write_program(bin.path(), "probe-other", 0o601);
    write_program(bin.path(), "probe-plain", 0o644);
    assert_eq!(
        executables_in(&search_path([bin.path()]), "probe", &[".exe"]),
        ["probe-other", "probe-owner"]
    );
}

#[test]
fn executables_in_filters_by_prefix() {
    let bin = tempfile::tempdir().unwrap();
    write_program(bin.path(), "alpha", 0o755);
    write_program(bin.path(), "beta", 0o755);
    assert_eq!(
        executables_in(&search_path([bin.path()]), "al", &[".exe"]),
        ["alpha"]
    );
}

#[test]
fn executables_in_follows_a_symlink_to_a_program() {
    let bin = tempfile::tempdir().unwrap();
    let real = write_program(bin.path(), "real", 0o755);
    let plain = write_program(bin.path(), "plain", 0o644);
    std::os::unix::fs::symlink(&real, bin.path().join("link-to-real")).unwrap();
    std::os::unix::fs::symlink(&plain, bin.path().join("link-to-plain")).unwrap();
    std::os::unix::fs::symlink(bin.path().join("gone"), bin.path().join("link-dangling")).unwrap();
    assert_eq!(
        executables_in(&search_path([bin.path()]), "link", &[".exe"]),
        ["link-to-real"]
    );
}

#[test]
fn executables_in_lists_a_name_on_two_directories_once() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    write_program(first.path(), "tool", 0o644);
    write_program(second.path(), "tool", 0o755);
    write_program(second.path(), "toolkit", 0o755);
    assert_eq!(
        executables_in(
            &search_path([first.path(), second.path(), second.path()]),
            "tool",
            &[".exe"]
        ),
        ["tool", "toolkit"]
    );
}
