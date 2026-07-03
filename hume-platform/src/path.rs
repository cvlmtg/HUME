//! Cross-platform path utilities.
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
use std::path::{Component, Path, PathBuf};

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
pub fn expand(s: &str) -> Cow<'_, str> {
    expand_with(s, |k| std::env::var(k), crate::dirs::home_dir)
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
            if let Some(after_close) = after_name.strip_prefix('}') {
                match env_lookup(name) {
                    Ok(val) => out.push_str(&val),
                    Err(_) => {
                        out.push_str("${");
                        out.push_str(name);
                        out.push('}');
                    }
                }
                remaining = after_close;
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
                Err(_) => {
                    out.push('$');
                    out.push_str(name);
                }
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
            Err(_) => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
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

// ── Home shortening ───────────────────────────────────────────────────────────

/// Replace the user's home directory prefix with `~` for display.
///
/// `"/home/user/dev/hume"` → `"~/dev/hume"`.  Exact match returns `"~"`.
/// When `home_dir()` is unavailable or the path does not start with the home
/// directory, the full path is returned unchanged.
pub fn shorten_home(path: &std::path::Path) -> String {
    shorten_home_with(path, crate::dirs::home_dir)
}

fn shorten_home_with(
    path: &std::path::Path,
    home_fn: impl FnOnce() -> Option<std::path::PathBuf>,
) -> String {
    if let Some(home) = home_fn() {
        match path.strip_prefix(&home) {
            Ok(suffix) if suffix.as_os_str().is_empty() => "~".to_string(),
            Ok(suffix) => format!("~{}{}", std::path::MAIN_SEPARATOR, suffix.display()),
            Err(_) => path.display().to_string(),
        }
    } else {
        path.display().to_string()
    }
}

// ── Separator utilities ───────────────────────────────────────────────────────

/// Returns `true` if `c` is a path-component separator on the current platform.
///
/// Always true for `/`.  Also true for `\` on Windows, where `\` cannot appear
/// in a filename and is therefore unambiguously a separator.  On Unix `\` is a
/// valid filename character and must **not** be treated as a separator.
pub fn is_path_sep(c: char) -> bool {
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
pub fn split_path_at_sep(s: &str) -> (&str, &str) {
    match s.rfind(is_path_sep) {
        Some(i) => (&s[..=i], &s[i + 1..]),
        None => ("", s),
    }
}

// ── Windows UNC prefix ───────────────────────────────────────────────────────

/// Strip the `\\?\` extended-length prefix from a Windows path so that the
/// result is a plain drive-letter path (e.g. `C:\Users\…\hume`).
///
/// Plain drive paths accept forward slashes from Scheme's `string-append`;
/// `\\?\`-prefixed paths go through the NT object manager directly and are
/// strict about backslashes.  Scheme plugins build paths via `(path-join …)`
/// which uses the native separator, but the display form must be prefix-free
/// so that even old-style string concatenation doesn't produce malformed paths.
///
/// Only strips verbatim drive prefixes (`\\?\C:\…`).  Verbatim UNC paths
/// (`\\?\UNC\…`) are left unchanged; they are rare and the `\\` prefix they
/// collapse to is already a valid UNC path.
///
/// On non-Windows targets this is a no-op.
#[cfg(windows)]
pub fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    const VERBATIM: &str = r"\\?\";
    match p.to_str() {
        Some(s) if s.starts_with(VERBATIM) && !s[VERBATIM.len()..].starts_with("UNC\\") => {
            PathBuf::from(&s[VERBATIM.len()..])
        }
        _ => p,
    }
}

#[cfg(not(windows))]
#[inline]
pub fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    p
}

// ── Path safety helpers ───────────────────────────────────────────────────────

/// Returns `true` if `path` contains any `..` (`ParentDir`) components.
///
/// Used for write-path operations where the target may not yet exist — we
/// cannot call `canonicalize` on a non-existent path, so `..` components are
/// rejected explicitly before a `starts_with` prefix check.
pub fn has_dotdot(path: &Path) -> bool {
    path.components().any(|c| c == Component::ParentDir)
}

/// Make `typed` absolute by joining it against `cwd` when relative, then
/// normalize `.` and `..` lexically without touching the filesystem.
///
/// Symlinks are intentionally **not** resolved — this is for display purposes
/// where the user expects to see the path as they typed it (e.g. inside a
/// symlinked project root).
pub fn absolute_unresolved(typed: &Path, cwd: &Path) -> PathBuf {
    let joined = if typed.is_relative() {
        cwd.join(typed)
    } else {
        typed.to_owned()
    };
    normalize_lexical(&joined)
}

/// Normalize a path lexically (without filesystem access) by collapsing `.`
/// and `..` components.
///
/// **Not a security substitute for `canonicalize`** (symlinks are not
/// resolved).  Safe to use only when combined with an explicit `..`-rejection
/// check via [`has_dotdot`].
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Returns `true` if `s` is a valid, safe filesystem path segment.
///
/// A valid segment is:
/// - Non-empty
/// - Not `.` or `..`
/// - Contains no `/`, `\`, `"`, or NUL character
///
/// Dots elsewhere are permitted (e.g. `v1.2.3`).  The `\` and `"` rejections
/// are cross-platform intentional: on Unix both are technically valid filename
/// characters, but `\` is unsafe in portable paths and `"` is invalid in
/// Windows filenames (and unsafe to embed in quoted contexts).
///
/// Used to validate plugin-name components and path arguments before they are
/// joined onto sandboxed base directories.
pub fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c != '/' && c != '\\' && c != '"' && c != '\0')
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

    fn no_home() -> Option<PathBuf> {
        None
    }

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
    #[cfg(not(windows))]
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
    #[cfg(not(windows))]
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

    // ── Windows env-var expansion ─────────────────────────────────────────────

    #[test]
    #[cfg(windows)]
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
    #[cfg(windows)]
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

    // ── shorten_home_with ─────────────────────────────────────────────────────

    #[cfg(not(windows))]
    #[test]
    fn shorten_home_with_path_inside_home() {
        use std::path::Path;
        let got = shorten_home_with(Path::new("/home/user/dev/hume"), || {
            Some(PathBuf::from("/home/user"))
        });
        assert_eq!(got, "~/dev/hume");
    }

    #[cfg(not(windows))]
    #[test]
    fn shorten_home_with_path_equal_to_home() {
        use std::path::Path;
        let got = shorten_home_with(Path::new("/home/user"), || {
            Some(PathBuf::from("/home/user"))
        });
        assert_eq!(got, "~");
    }

    #[cfg(not(windows))]
    #[test]
    fn shorten_home_with_path_outside_home() {
        use std::path::Path;
        let got = shorten_home_with(Path::new("/tmp/foo"), || Some(PathBuf::from("/home/user")));
        assert_eq!(got, "/tmp/foo");
    }

    #[cfg(not(windows))]
    #[test]
    fn shorten_home_with_no_home_returns_full_path() {
        use std::path::Path;
        let got = shorten_home_with(Path::new("/tmp/foo"), || None);
        assert_eq!(got, "/tmp/foo");
    }

    #[cfg(not(windows))]
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

    // ── has_dotdot ────────────────────────────────────────────────────────────

    #[test]
    fn has_dotdot_detects_bare_parent() {
        assert!(has_dotdot(Path::new("..")));
    }

    #[test]
    fn has_dotdot_detects_mid_path_parent() {
        assert!(has_dotdot(Path::new("foo/../bar")));
    }

    #[test]
    fn has_dotdot_does_not_flag_cur_dir() {
        assert!(!has_dotdot(Path::new(".")));
        assert!(!has_dotdot(Path::new("foo/./bar")));
    }

    #[test]
    fn has_dotdot_clean_path_is_false() {
        assert!(!has_dotdot(Path::new("foo/bar/baz")));
    }

    // ── normalize_lexical ─────────────────────────────────────────────────────

    #[test]
    fn normalize_lexical_removes_cur_dir() {
        assert_eq!(normalize_lexical(Path::new("a/./b")), PathBuf::from("a/b"));
    }

    #[test]
    fn normalize_lexical_pops_parent_dir() {
        assert_eq!(
            normalize_lexical(Path::new("a/b/../c")),
            PathBuf::from("a/c")
        );
    }

    #[test]
    fn normalize_lexical_pop_on_empty_is_safe() {
        // Leading ".." when the output is empty: pop() on an empty PathBuf is a
        // no-op, so the ".." is silently discarded and only "a" survives.
        assert_eq!(normalize_lexical(Path::new("../a")), PathBuf::from("a"));
    }

    #[test]
    fn normalize_lexical_normal_path_unchanged() {
        assert_eq!(
            normalize_lexical(Path::new("a/b/c")),
            PathBuf::from("a/b/c")
        );
    }

    // ── is_safe_segment ───────────────────────────────────────────────────────

    #[test]
    fn is_safe_segment_empty_is_rejected() {
        assert!(!is_safe_segment(""));
    }

    #[test]
    fn is_safe_segment_dot_is_rejected() {
        assert!(!is_safe_segment("."));
    }

    #[test]
    fn is_safe_segment_dotdot_is_rejected() {
        assert!(!is_safe_segment(".."));
    }

    #[test]
    fn is_safe_segment_forward_slash_is_rejected() {
        assert!(!is_safe_segment("a/b"));
    }

    #[test]
    fn is_safe_segment_backslash_is_rejected() {
        assert!(!is_safe_segment(r"a\b"));
    }

    #[test]
    fn is_safe_segment_nul_is_rejected() {
        assert!(!is_safe_segment("a\0b"));
    }

    #[test]
    fn is_safe_segment_quote_is_rejected() {
        assert!(!is_safe_segment("a\"b"));
    }

    #[test]
    fn is_safe_segment_version_string_is_valid() {
        // Dots within a segment are fine (e.g. "v1.2.3").
        assert!(is_safe_segment("v1.2.3"));
    }

    #[test]
    fn is_safe_segment_plain_name_is_valid() {
        assert!(is_safe_segment("helix-surround"));
    }

    // ── absolute_unresolved ───────────────────────────────────────────────────

    #[test]
    fn absolute_unresolved_relative_joins_cwd() {
        let cwd = PathBuf::from("/home/user/projects");
        let result = absolute_unresolved(std::path::Path::new("foo/bar.txt"), &cwd);
        assert_eq!(result, PathBuf::from("/home/user/projects/foo/bar.txt"));
    }

    #[test]
    fn absolute_unresolved_absolute_is_unchanged() {
        let cwd = PathBuf::from("/home/user/projects");
        let result = absolute_unresolved(std::path::Path::new("/tmp/foo.txt"), &cwd);
        assert_eq!(result, PathBuf::from("/tmp/foo.txt"));
    }

    #[test]
    fn absolute_unresolved_collapses_dots() {
        let cwd = PathBuf::from("/home/user");
        // `./foo/../bar` → `/home/user/bar`
        let result = absolute_unresolved(std::path::Path::new("./foo/../bar.txt"), &cwd);
        assert_eq!(result, PathBuf::from("/home/user/bar.txt"));
    }

    #[test]
    fn absolute_unresolved_does_not_canonicalize_symlinks() {
        // We can't make a symlink in a pure unit test, but we can verify that
        // a directory name that looks like it could be a symlink (e.g. "link")
        // is passed through unchanged — no fs access occurs.
        let cwd = PathBuf::from("/real/path");
        let result = absolute_unresolved(std::path::Path::new("../symlink-dir/file.txt"), &cwd);
        // `../` from `/real/path` → `/real` (lexical pop), then `symlink-dir/file.txt`
        assert_eq!(result, PathBuf::from("/real/symlink-dir/file.txt"));
    }
}
