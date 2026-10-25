//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;

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

// ── completion_entries ────────────────────────────────────────────────────

#[test]
fn completion_entries_lists_a_link_as_what_it_points_at() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("real")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("missing"), dir.path().join("dangling")).unwrap();
    assert_eq!(
        completion_entries(dir.path(), "", false),
        ["dangling", "link/", "real/"]
    );
    assert_eq!(completion_entries(dir.path(), "", true), ["link/", "real/"]);
}

#[test]
fn completion_entries_matches_the_prefix_in_its_own_case() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.txt"), b"").unwrap();
    assert_eq!(completion_entries(dir.path(), "pro", false), ["probe.txt"]);
    assert!(completion_entries(dir.path(), "PRO", false).is_empty());
}

// ── absolute ──────────────────────────────────────────────────────────────

#[test]
fn absolute_joins_a_relative_path_onto_cwd() {
    let got = absolute(Path::new("foo/bar.txt"), Path::new("/home/user")).unwrap();
    assert_eq!(got, Path::new("/home/user/foo/bar.txt"));
}

#[test]
fn absolute_keeps_an_absolute_path() {
    let got = absolute(Path::new("/tmp/foo.txt"), Path::new("/home/user")).unwrap();
    assert_eq!(got, Path::new("/tmp/foo.txt"));
}

#[test]
fn absolute_collapses_dot_and_repeated_and_trailing_separators() {
    let got = absolute(Path::new(".//foo/./bar/"), Path::new("/home/user")).unwrap();
    assert_eq!(got.as_os_str(), "/home/user/foo/bar");
}

/// `a/link` points at `b/c`, so `a/link/..` is `b`, as the OS reads it.
#[test]
fn absolute_steps_out_of_a_symlink_target_on_dot_dot() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("a")).unwrap();
    std::fs::create_dir_all(dir.path().join("b/c")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("b/c"), dir.path().join("a/link")).unwrap();

    let got = absolute(Path::new("a/link/../f.txt"), dir.path()).unwrap();

    let b = std::fs::canonicalize(dir.path().join("b")).unwrap();
    assert_eq!(got, b.join("f.txt"));
}

#[test]
fn absolute_keeps_a_symlink_after_the_last_dot_dot() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("a/real")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("a/real"), dir.path().join("a/link")).unwrap();

    let got = absolute(Path::new("a/../a/link/f.txt"), dir.path()).unwrap();

    let a = std::fs::canonicalize(dir.path().join("a")).unwrap();
    assert_eq!(got, a.join("link/f.txt"));
}

#[test]
fn absolute_fails_for_a_dot_dot_after_a_missing_directory() {
    let dir = tempfile::tempdir().unwrap();
    let err = absolute(Path::new("missing/../f.txt"), dir.path()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}
