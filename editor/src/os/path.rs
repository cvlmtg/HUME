//! Cross-platform path-separator utilities for minibuffer completion.
//!
//! On Unix the only path separator is `/`.  On Windows both `/` and `\` are
//! accepted by the OS; we recognise either when parsing user input but always
//! emit `/` in completion replacements (Windows file APIs accept forward
//! slashes, and it keeps the rest of the completion logic uniform).

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
    use super::*;

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
