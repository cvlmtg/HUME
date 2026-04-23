//! Cross-platform path utilities for minibuffer completion.
//!
//! On Unix the only path separator is `/`.  On Windows both `/` and `\` are
//! accepted by the OS; we recognise either when parsing user input but always
//! emit `/` in completion replacements (Windows file APIs accept forward
//! slashes, and it keeps the rest of the completion logic uniform).
//!
//! This module also provides `expand`, which applies shell-style tilde and
//! environment-variable expansion to user-supplied path strings before they
//! are resolved by the filesystem.  Env-var syntax is native per platform:
//! `$VAR` / `${VAR}` on Unix, `%VAR%` on Windows.

use std::borrow::Cow;

// ── Path expansion ────────────────────────────────────────────────────────────

/// Platform env-var sigil: `$` on Unix, `%` on Windows.
#[cfg(not(windows))]
const ENV_SIGIL: char = '$';
#[cfg(windows)]
const ENV_SIGIL: char = '%';

/// Expand shell-style tilde and environment-variable references in a
/// user-supplied path string.
///
/// **Tilde**: A leading `~` followed by a path separator or end-of-string is
/// replaced by the current user's home directory (`$HOME` on Unix,
/// `%USERPROFILE%` on Windows).  `~user/foo` forms are **not** expanded (no
/// `getpwnam` lookup).
///
/// **Env vars**: Native syntax per platform — `$VAR` / `${VAR}` on Unix;
/// `%VAR%` on Windows.  Unknown variables are left **literal** (not replaced
/// with an empty string) so that mistyped `$NONEXISTENT/foo` produces a
/// recognisable "no such file" error rather than silently resolving to `/foo`.
///
/// Returns `Cow::Borrowed(s)` unchanged when no expansion is needed, avoiding
/// allocation for the common case of plain absolute or relative paths.
pub(crate) fn expand(s: &str) -> Cow<'_, str> {
    expand_with(s, |k| std::env::var(k), crate::os::dirs::home_dir)
}

/// Testable core of [`expand`].
///
/// `env_lookup` mirrors `std::env::var`.  `home_fn` is called at most once to
/// obtain the home directory for tilde expansion.
fn expand_with(
    s: &str,
    env_lookup: impl Fn(&str) -> Result<String, std::env::VarError>,
    home_fn: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Cow<'_, str> {
    if !needs_expansion(s) {
        return Cow::Borrowed(s);
    }

    let mut out = String::with_capacity(s.len() + 16);
    let mut rest = s;

    // Stage 1: tilde
    if let Some(tail) = strip_tilde(rest) {
        if let Some(home) = home_fn() {
            out.push_str(&home.to_string_lossy());
        } else {
            out.push('~'); // HOME unset — pass through literally
        }
        rest = tail; // tail is the portion after `~` (may start with `/`)
    }

    // Stage 2: env vars
    expand_env_vars(rest, &env_lookup, &mut out);

    Cow::Owned(out)
}

fn needs_expansion(s: &str) -> bool {
    s.starts_with('~') || s.contains(ENV_SIGIL)
}

/// If `s` begins with an expandable `~` (i.e. `~` alone, `~/`, or `~\` on
/// Windows), returns the slice **after** the `~`.  Returns `None` for `~user`
/// and any string not starting with `~`.
fn strip_tilde(s: &str) -> Option<&str> {
    if !s.starts_with('~') {
        return None;
    }
    let after = &s[1..];
    if after.is_empty() || after.starts_with('/') || (cfg!(windows) && after.starts_with('\\')) {
        Some(after)
    } else {
        None // `~user` form — do not expand
    }
}

/// Scan `s` for env-var references (Unix `$VAR`/`${VAR}` syntax) and append
/// the expanded result to `out`.  Unknown variables are emitted literally.
#[cfg(not(windows))]
fn expand_env_vars(
    s: &str,
    env_lookup: &impl Fn(&str) -> Result<String, std::env::VarError>,
    out: &mut String,
) {
    let mut remaining = s;
    while let Some(pos) = remaining.find('$') {
        out.push_str(&remaining[..pos]);
        remaining = &remaining[pos + 1..]; // slice starts just after `$`

        if remaining.is_empty() {
            out.push('$');
            break;
        }

        if remaining.starts_with('{') {
            // `${NAME}` form
            let after_brace = &remaining[1..];
            let nlen = var_name_len(after_brace);
            if nlen == 0 {
                out.push_str("${");
                remaining = after_brace;
                continue;
            }
            let name = &after_brace[..nlen];
            let after_name = &after_brace[nlen..];
            if after_name.starts_with('}') {
                match env_lookup(name) {
                    Ok(val) => out.push_str(&val),
                    Err(_) => { out.push_str("${"); out.push_str(name); out.push('}'); }
                }
                remaining = &after_name[1..]; // skip `}`
            } else {
                // Unclosed `${...` — literal.
                out.push_str("${");
                out.push_str(name);
                remaining = after_name;
            }
        } else {
            // `$NAME` form
            let nlen = var_name_len(remaining);
            if nlen == 0 {
                out.push('$');
                // `remaining` already points past the `$`; keep scanning.
                continue;
            }
            let name = &remaining[..nlen];
            match env_lookup(name) {
                Ok(val) => out.push_str(&val),
                Err(_) => { out.push('$'); out.push_str(name); }
            }
            remaining = &remaining[nlen..];
        }
    }
    out.push_str(remaining);
}

/// Scan `s` for env-var references (Windows `%VAR%` syntax) and append the
/// expanded result to `out`.  Unknown variables are emitted literally.
#[cfg(windows)]
fn expand_env_vars(
    s: &str,
    env_lookup: &impl Fn(&str) -> Result<String, std::env::VarError>,
    out: &mut String,
) {
    let mut remaining = s;
    while let Some(pos) = remaining.find('%') {
        out.push_str(&remaining[..pos]);
        remaining = &remaining[pos + 1..]; // slice starts just after opening `%`

        let nlen = var_name_len(remaining);
        // Need a non-empty name AND a closing `%`.
        if nlen == 0 || !remaining[nlen..].starts_with('%') {
            out.push('%'); // lone or unclosed `%` — literal
            // `remaining` already past the opening `%`; keep scanning for next.
            continue;
        }
        let name = &remaining[..nlen];
        match env_lookup(name) {
            Ok(val) => out.push_str(&val),
            Err(_) => { out.push('%'); out.push_str(name); out.push('%'); }
        }
        remaining = &remaining[nlen + 1..]; // skip name + closing `%`
    }
    out.push_str(remaining);
}

/// Length in bytes of the leading `[A-Za-z_][A-Za-z0-9_]*` identifier in `s`.
/// Returns `0` if `s` does not start with a valid identifier character.
fn var_name_len(s: &str) -> usize {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i == 0 && !(c.is_ascii_alphabetic() || c == '_') {
            return 0;
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

// ── Separator utilities ───────────────────────────────────────────────────────

/// Returns `true` if `c` is a path-component separator on the current platform.
///
/// Always true for `/`.  Also true for `\` on Windows, where `\` cannot appear
/// in a filename and is therefore unambiguously a separator.  On Unix `\` is a
/// valid filename character and must **not** be treated as a separator.
pub(crate) fn is_path_sep(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// Split a path prefix at the last separator, returning `(dir, filename_prefix)`.
///
/// `dir` includes the trailing separator so it can be used directly as a
/// prefix when building completion replacements.  If there is no separator,
/// `dir` is `""` and `filename_prefix` is the whole string.
///
/// `split_path_at_sep("foo/bar")` → `("foo/", "bar")`;
/// `split_path_at_sep("/tmp/")` → `("/tmp/", "")`;
/// `split_path_at_sep("foo")` → `("", "foo")`.
pub(crate) fn split_path_at_sep(s: &str) -> (&str, &str) {
    match s.rfind(is_path_sep) {
        Some(i) => (&s[..=i], &s[i + 1..]),
        None    => ("", s),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    // ── expand_with ───────────────────────────────────────────────────────────

    fn no_env(_: &str) -> Result<String, std::env::VarError> {
        Err(std::env::VarError::NotPresent)
    }

    fn home(h: &'static str) -> impl FnOnce() -> Option<PathBuf> {
        move || Some(PathBuf::from(h))
    }

    fn no_home() -> Option<PathBuf> { None }

    #[test]
    fn expand_absolute_path_is_borrowed() {
        let result = expand_with("/abs/path", no_env, no_home);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result, "/abs/path");
    }

    #[test]
    fn expand_relative_path_is_borrowed() {
        let result = expand_with("relative/path", no_env, no_home);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result, "relative/path");
    }

    #[test]
    fn expand_tilde_alone() {
        let result = expand_with("~", no_env, home("/home/user"));
        assert_eq!(result, "/home/user");
    }

    #[test]
    fn expand_tilde_slash() {
        let result = expand_with("~/foo", no_env, home("/home/user"));
        assert_eq!(result, "/home/user/foo");
    }

    #[test]
    fn expand_tilde_user_form_unchanged() {
        let result = expand_with("~alice/foo", no_env, home("/home/user"));
        // `~alice` is not expanded (no getpwnam).
        assert_eq!(result, "~alice/foo");
    }

    #[test]
    fn expand_tilde_mid_string_unchanged() {
        let result = expand_with("/foo~bar", no_env, no_home);
        assert_eq!(result, "/foo~bar");
    }

    #[test]
    fn expand_tilde_home_unset_leaves_literal() {
        let result = expand_with("~/foo", no_env, no_home);
        assert_eq!(result, "~/foo");
    }

    // ── Unix env-var expansion ────────────────────────────────────────────────

    #[test]
    #[cfg(not(windows))]
    fn expand_dollar_var() {
        let result = expand_with("$HOME/foo", |k| {
            if k == "HOME" { Ok("/h".into()) } else { Err(std::env::VarError::NotPresent) }
        }, no_home);
        assert_eq!(result, "/h/foo");
    }

    #[test]
    #[cfg(not(windows))]
    fn expand_dollar_braced_var() {
        let result = expand_with("${HOME}/foo", |k| {
            if k == "HOME" { Ok("/h".into()) } else { Err(std::env::VarError::NotPresent) }
        }, no_home);
        assert_eq!(result, "/h/foo");
    }

    #[test]
    #[cfg(not(windows))]
    fn expand_unknown_var_stays_literal() {
        let result = expand_with("$NONEXISTENT/foo", no_env, no_home);
        assert_eq!(result, "$NONEXISTENT/foo");
    }

    #[test]
    #[cfg(not(windows))]
    fn expand_unclosed_brace_stays_literal() {
        let result = expand_with("${UNCLOSED/foo", no_env, no_home);
        // `${UNCLOSED` has no closing `}` before `/` — emitted literally.
        assert_eq!(result, "${UNCLOSED/foo");
    }

    #[test]
    #[cfg(not(windows))]
    fn expand_trailing_dollar_is_literal() {
        let result = expand_with("/foo$", no_env, no_home);
        assert_eq!(result, "/foo$");
    }

    #[test]
    #[cfg(not(windows))]
    fn expand_dollar_digit_is_literal() {
        // `$1var` — digit cannot start an identifier.
        let result = expand_with("$1var", no_env, no_home);
        assert_eq!(result, "$1var");
    }

    #[test]
    #[cfg(not(windows))]
    fn expand_multiple_vars() {
        let result = expand_with("$A/$B", |k| match k {
            "A" => Ok("x".into()),
            "B" => Ok("y".into()),
            _   => Err(std::env::VarError::NotPresent),
        }, no_home);
        assert_eq!(result, "x/y");
    }

    #[test]
    #[cfg(not(windows))]
    fn expand_tilde_and_env_var() {
        let result = expand_with("~/$DIR/file", |k| {
            if k == "DIR" { Ok("docs".into()) } else { Err(std::env::VarError::NotPresent) }
        }, home("/h"));
        assert_eq!(result, "/h/docs/file");
    }

    // ── Windows env-var expansion ─────────────────────────────────────────────

    #[test]
    #[cfg(windows)]
    fn expand_percent_var() {
        let result = expand_with(r"%USERPROFILE%\foo", |k| {
            if k == "USERPROFILE" { Ok(r"C:\Users\Alice".into()) } else { Err(std::env::VarError::NotPresent) }
        }, no_home);
        assert_eq!(result, r"C:\Users\Alice\foo");
    }

    #[test]
    #[cfg(windows)]
    fn expand_unknown_percent_var_stays_literal() {
        let result = expand_with(r"%NONEXISTENT%\foo", no_env, no_home);
        assert_eq!(result, r"%NONEXISTENT%\foo");
    }

    #[test]
    #[cfg(windows)]
    fn expand_unclosed_percent_stays_literal() {
        let result = expand_with(r"%UNCLOSED\foo", no_env, no_home);
        assert_eq!(result, r"%UNCLOSED\foo");
    }

    #[test]
    #[cfg(windows)]
    fn expand_consecutive_percent_vars() {
        let result = expand_with("%A%%B%", |k| match k {
            "A" => Ok("x".into()),
            "B" => Ok("y".into()),
            _   => Err(std::env::VarError::NotPresent),
        }, no_home);
        assert_eq!(result, "xy");
    }

    #[test]
    #[cfg(windows)]
    fn expand_tilde_and_percent_var() {
        let result = expand_with(r"~\%DIR%\file", |k| {
            if k == "DIR" { Ok("docs".into()) } else { Err(std::env::VarError::NotPresent) }
        }, home(r"C:\Users\Alice"));
        assert_eq!(result, r"C:\Users\Alice\docs\file");
    }

    // ── split_path_at_sep ─────────────────────────────────────────────────────

    #[test]
    fn split_simple() {
        assert_eq!(split_path_at_sep("foo/bar"), ("foo/", "bar"));
    }

    #[test]
    fn split_absolute() {
        assert_eq!(split_path_at_sep("/tmp/alpha"), ("/tmp/", "alpha"));
    }

    #[test]
    fn split_trailing_sep() {
        assert_eq!(split_path_at_sep("/tmp/alpha/"), ("/tmp/alpha/", ""));
    }

    #[test]
    fn split_no_sep() {
        assert_eq!(split_path_at_sep("foo"), ("", "foo"));
    }

    #[test]
    fn split_empty() {
        assert_eq!(split_path_at_sep(""), ("", ""));
    }

    #[test]
    fn split_root_only() {
        assert_eq!(split_path_at_sep("/"), ("/", ""));
    }

    #[test]
    fn is_path_sep_forward_slash() {
        assert!(is_path_sep('/'));
    }

    #[test]
    fn is_path_sep_regular_chars() {
        assert!(!is_path_sep('a'));
        assert!(!is_path_sep('.'));
        assert!(!is_path_sep(' '));
    }
}
