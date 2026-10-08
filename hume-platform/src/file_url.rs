//! Absolute path → `file://` URL, shared by the LSP URIs and the terminal's
//! OSC 7 working-directory report.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileUrlError {
    /// The input path was not absolute.
    NotAbsolute,
    /// The input path is not valid UTF-8, never silently mangled via a lossy
    /// conversion.
    NotUtf8,
}

/// Absolute path → `file://{host}/…` URL. `host` becomes the authority; empty
/// gives `file:///…`. Percent-encodes everything but unreserved chars
/// (`A-Za-z0-9-._~`), `/`, and `:` (pchar-legal, left bare so a drive letter
/// reads `C:` not `C%3A`), in `host` as in the path. Windows: any `\\?\`
/// verbatim prefix is stripped first; a UNC path (`\\server\share\…` or
/// `\\?\UNC\server\share\…`) emits `server` as the authority
/// (`file://server/share/…`, `host` ignored) instead of folding it into the
/// path; otherwise backslashes become `/` and a drive letter gets a synthetic
/// leading `/` so the result reads `file://{host}/C:/…`.
///
/// # Errors
/// [`FileUrlError::NotAbsolute`] if `path` is relative, never joined against
/// a cwd; the caller owns canonicalization. [`FileUrlError::NotUtf8`] if
/// `path` is not valid UTF-8.
pub fn path_to_file_url(path: &Path, host: &str) -> Result<String, FileUrlError> {
    if !path.is_absolute() {
        return Err(FileUrlError::NotAbsolute);
    }

    let path_str = path.to_str().ok_or(FileUrlError::NotUtf8)?;
    #[cfg(windows)]
    let stripped = crate::path::strip_verbatim(path).unwrap_or(path_str);
    #[cfg(not(windows))]
    let stripped = path_str;

    #[cfg(windows)]
    if let Some((unc_host, rest)) = unc_host_and_rest(stripped) {
        let with_leading_slash = ensure_leading_slash(normalize_separators(rest));
        return Ok(format!(
            "file://{}{}",
            percent_encode_path(unc_host),
            percent_encode_path(&with_leading_slash)
        ));
    }

    let with_leading_slash = ensure_leading_slash(normalize_separators(stripped));
    Ok(format!(
        "file://{}{}",
        percent_encode_path(host),
        percent_encode_path(&with_leading_slash)
    ))
}

/// Backslash → `/`, so a Windows path reads as a URI path. A no-op on
/// Unix, where `\` is an ordinary, legal filename byte that must round-trip
/// untouched (percent-encoded on the way out, accepted verbatim back in).
#[cfg(windows)]
pub fn normalize_separators(s: &str) -> String {
    s.chars().map(|c| if c == '\\' { '/' } else { c }).collect()
}

#[cfg(not(windows))]
pub fn normalize_separators(s: &str) -> String {
    s.to_owned()
}

/// Prefixes `s` with `/` unless it already starts with one: a URI path
/// component always needs the leading separator, whether it came from a
/// drive-letter path (`C:/foo` -> `/C:/foo`) or a UNC share's tail.
fn ensure_leading_slash(s: String) -> String {
    if s.starts_with('/') {
        s
    } else {
        format!("/{s}")
    }
}

/// `\\server\share\rest` or `\\?\UNC\server\share\rest` -> `(server,
/// "share\rest")`; `None` for anything else, including a plain local
/// `\\?\`-verbatim path (already stripped by [`crate::path::strip_verbatim`]
/// before this runs).
#[cfg(windows)]
fn unc_host_and_rest(s: &str) -> Option<(&str, &str)> {
    let rest = s
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| s.strip_prefix(r"\\"))?;
    rest.split_once('\\')
}

/// `true` for RFC 3986 unreserved characters.
fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Percent-encode every byte except unreserved chars, `/`, and `:`, the
/// only bytes [`path_to_file_url`] leaves unencoded. `:` is pchar-legal per
/// RFC 3986 (not "unreserved", but still fine bare in a path segment), left
/// bare so a Windows drive letter reads `C:` rather than `C%3A`. Operates
/// byte-wise (not char-wise) so multi-byte UTF-8 sequences encode correctly:
/// each of their bytes is non-unreserved and gets its own `%XX`.
fn percent_encode_path(path_str: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(path_str.len());
    for b in path_str.bytes() {
        if is_unreserved(b) || matches!(b, b'/' | b':') {
            out.push(b as char);
        } else {
            write!(out, "%{b:02X}").expect("writing to a String cannot fail");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn puts_the_host_in_the_authority() {
        assert_eq!(
            path_to_file_url(Path::new("/tmp/a b"), "box").as_deref(),
            Ok("file://box/tmp/a%20b")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn percent_encodes_the_host() {
        assert_eq!(
            path_to_file_url(Path::new("/tmp"), "my box").as_deref(),
            Ok("file://my%20box/tmp")
        );
    }

    #[test]
    fn rejects_relative_path() {
        assert_eq!(
            path_to_file_url(Path::new("rel/dir"), "box"),
            Err(FileUrlError::NotAbsolute)
        );
    }

    #[cfg(windows)]
    #[test]
    fn puts_the_host_before_a_drive_letter_path() {
        assert_eq!(
            path_to_file_url(Path::new(r"D:\test"), "box").as_deref(),
            Ok("file://box/D:/test")
        );
    }

    #[cfg(windows)]
    #[test]
    fn keeps_a_unc_server_as_the_authority() {
        assert_eq!(
            path_to_file_url(Path::new(r"\\srv\share\x"), "box").as_deref(),
            Ok("file://srv/share/x")
        );
    }
}
