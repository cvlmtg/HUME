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
/// - Contains no `/`, `\`, `"`, `:`, or NUL character
///
/// Dots elsewhere are permitted (e.g. `v1.2.3`).  The `\`, `"`, and `:`
/// rejections are cross-platform intentional: on Unix all three are
/// technically valid filename characters, but `\` is unsafe in portable
/// paths, `"` is invalid in Windows filenames (and unsafe to embed in quoted
/// contexts), and `:` after a single letter makes `PathBuf::push` treat the
/// segment as a drive-relative root on Windows — replacing the sandboxed base
/// path entirely instead of joining onto it (e.g. `c:evil` under
/// `data_dir/plugins/`).
///
/// Used to validate plugin-name components and path arguments before they are
/// joined onto sandboxed base directories.
pub fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c != '/' && c != '\\' && c != '"' && c != '\0' && c != ':')
}

#[cfg(test)]
mod tests;
