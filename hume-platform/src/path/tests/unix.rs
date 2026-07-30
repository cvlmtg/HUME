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
    // `${UNCLOSED` has no closing `}` before `/` — emitted literally.
    assert_eq!(result, "${UNCLOSED/foo");
}

#[test]
fn expand_trailing_dollar_is_literal() {
    let result = expand_with("/foo$", no_env, no_home);
    assert_eq!(result, "/foo$");
}

#[test]
fn expand_dollar_digit_is_literal() {
    // `$1var` — digit cannot start an identifier.
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
    // "/home/userx" must NOT match home="/home/user" — strip_prefix is
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
