//! Lossless, canonical path ↔ `file://` URI conversion.
//!
//! Outgoing URIs always come from the canonical `Buffer.path` (the SSOT, on
//! the `hume-editor` side); incoming URIs are converted to paths here and
//! canonicalized by the caller before buffer lookup — this module never
//! guesses a path on error, it only ever returns `Err`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// The URI's scheme is not `file` (or has no scheme at all).
    NotFileScheme,
    /// [`path_to_uri`]'s input path was not absolute.
    NotAbsolute,
    /// The URI's authority is present and is neither empty nor `localhost`.
    BadAuthority(String),
    /// The URI's path failed to percent-decode, or a decoded segment
    /// contains a path separator (a `%2F`/`%5C` disguising an extra
    /// boundary — rejected defensively, see [`uri_to_path`]).
    Decode(String),
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UriError::NotFileScheme => write!(f, "URI is not a file:// URI"),
            UriError::NotAbsolute => write!(f, "path is not absolute"),
            UriError::BadAuthority(host) => write!(f, "unsupported URI authority: {host:?}"),
            UriError::Decode(msg) => write!(f, "failed to decode URI path: {msg}"),
        }
    }
}

impl std::error::Error for UriError {}

/// Absolute path → `file://` URI. Percent-encodes everything but unreserved
/// chars (`A-Za-z0-9-._~`) and `/`. Windows: any `\\?\` verbatim prefix is
/// stripped first, backslashes become `/`, and a drive letter gets a
/// synthetic leading `/` so the result reads `file:///C:/…`.
///
/// # Errors
/// [`UriError::NotAbsolute`] if `path` is relative — never joined against a
/// cwd; the caller owns canonicalization.
pub fn path_to_uri(path: &Path) -> Result<lsp_types::Uri, UriError> {
    if !path.is_absolute() {
        return Err(UriError::NotAbsolute);
    }

    let path_str = path.to_string_lossy();
    let stripped = strip_verbatim_prefix(&path_str);
    let normalized: String = stripped
        .chars()
        .map(|c| if c == '\\' { '/' } else { c })
        .collect();
    let with_leading_slash = if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{normalized}")
    };

    let encoded = percent_encode_path(&with_leading_slash);
    let uri_str = format!("file://{encoded}");
    Ok(lsp_types::Uri::from_str(&uri_str)
        .expect("percent-encoded file URI is always syntactically valid"))
}

/// `file://` URI → absolute path. Accepts empty and `localhost` authority;
/// rejects other schemes/authorities loudly — never a guessed path.
///
/// # Errors
/// [`UriError::NotFileScheme`] for a non-`file` (or schemeless) URI,
/// [`UriError::BadAuthority`] for an authority that isn't empty or
/// `localhost`, [`UriError::Decode`] for a malformed or traversal-hazardous
/// path.
pub fn uri_to_path(uri: &lsp_types::Uri) -> Result<PathBuf, UriError> {
    let scheme = uri.scheme().ok_or(UriError::NotFileScheme)?;
    if !scheme.eq_lowercase("file") {
        return Err(UriError::NotFileScheme);
    }

    if let Some(authority) = uri.authority() {
        let host = authority.host().as_str();
        if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            return Err(UriError::BadAuthority(host.to_owned()));
        }
    }

    let mut segments = Vec::new();
    for raw_segment in uri.path().as_estr().split('/') {
        if raw_segment.as_str().is_empty() {
            continue; // the leading '/' of an absolute path produces one
        }
        let decoded = raw_segment
            .decode()
            .into_string()
            .map_err(|e| UriError::Decode(e.to_string()))?;
        if decoded.contains('/') || decoded.contains('\\') {
            return Err(UriError::Decode(format!(
                "segment {decoded:?} decodes to contain a path separator"
            )));
        }
        segments.push(decoded.into_owned());
    }

    // Windows drive-letter form: "file:///C:/foo" (or the colon escaped as
    // "file:///c%3A/foo") decodes to a first segment "C:" — join without a
    // leading separator so the result reads "C:\foo", not "\C:\foo".
    if let Some(first) = segments.first()
        && is_drive_letter_segment(first)
    {
        let mut path = first.clone();
        for seg in &segments[1..] {
            path.push('\\');
            path.push_str(seg);
        }
        return Ok(PathBuf::from(path));
    }

    let mut path = String::from("/");
    path.push_str(&segments.join("/"));
    Ok(PathBuf::from(path))
}

/// `true` if `segment` is a single ASCII letter followed by `:` (e.g. `"C:"`)
/// — the decoded shape of a Windows drive-letter path segment.
fn is_drive_letter_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// `true` for RFC 3986 unreserved characters — the only bytes [`path_to_uri`]
/// leaves unencoded besides `/`.
fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Percent-encode every byte except unreserved chars and `/`. Operates
/// byte-wise (not char-wise) so multi-byte UTF-8 sequences encode correctly
/// — each of their bytes is non-unreserved and gets its own `%XX`.
fn percent_encode_path(path_str: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(path_str.len());
    for b in path_str.bytes() {
        if is_unreserved(b) || b == b'/' {
            out.push(b as char);
        } else {
            write!(out, "%{b:02X}").expect("writing to a String cannot fail");
        }
    }
    out
}

/// Strip a `\\?\` verbatim prefix (but not a `\\?\UNC\` one), mirroring
/// `hume_platform::path::strip_unc_prefix`'s convention — this crate cannot
/// depend on `hume-platform`, so the pattern is duplicated rather than
/// imported.
#[cfg(windows)]
fn strip_verbatim_prefix(s: &str) -> &str {
    const VERBATIM: &str = r"\\?\";
    match s.strip_prefix(VERBATIM) {
        Some(rest) if !rest.starts_with("UNC\\") => rest,
        _ => s,
    }
}

#[cfg(not(windows))]
fn strip_verbatim_prefix(s: &str) -> &str {
    s
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(path: &Path) -> PathBuf {
        let uri = path_to_uri(path).expect("path_to_uri");
        uri_to_path(&uri).expect("uri_to_path")
    }

    // ── round-trip ───────────────────────────────────────────────────────────

    #[test]
    fn round_trip_plain_ascii_path() {
        let path = Path::new("/tmp/hello/world.rs");
        assert_eq!(round_trip(path), path);
    }

    #[test]
    fn round_trip_path_with_spaces() {
        let path = Path::new("/tmp/hello world/x file.rs");
        assert_eq!(round_trip(path), path);
    }

    #[test]
    fn round_trip_non_ascii_path() {
        let path = Path::new("/tmp/héllo/ö.rs");
        assert_eq!(round_trip(path), path);
    }

    #[test]
    fn round_trip_symbols_needing_escapes() {
        let path = Path::new("/tmp/a#b/c?d/e%f.rs");
        assert_eq!(round_trip(path), path);
    }

    #[test]
    fn path_to_uri_produces_expected_string_for_plain_path() {
        let uri = path_to_uri(Path::new("/tmp/x.rs")).expect("path_to_uri");
        assert_eq!(uri.as_str(), "file:///tmp/x.rs");
    }

    #[test]
    fn path_to_uri_encodes_percent_and_hash_and_question_and_space() {
        let uri = path_to_uri(Path::new("/tmp/a b#c?d%e.rs")).expect("path_to_uri");
        assert_eq!(uri.as_str(), "file:///tmp/a%20b%23c%3Fd%25e.rs");
    }

    // ── path_to_uri: relative rejected ──────────────────────────────────────

    #[test]
    fn path_to_uri_rejects_relative_path() {
        assert_eq!(
            path_to_uri(Path::new("relative/path.rs")),
            Err(UriError::NotAbsolute)
        );
    }

    // ── uri_to_path: inbound parsing ────────────────────────────────────────

    #[test]
    fn uri_to_path_drive_letter_plain_colon() {
        let uri = lsp_types::Uri::from_str("file:///C:/x").expect("parse");
        assert_eq!(
            uri_to_path(&uri).expect("uri_to_path"),
            PathBuf::from("C:\\x")
        );
    }

    #[test]
    fn uri_to_path_drive_letter_escaped_colon() {
        let uri = lsp_types::Uri::from_str("file:///c%3A/x").expect("parse");
        assert_eq!(
            uri_to_path(&uri).expect("uri_to_path"),
            PathBuf::from("c:\\x")
        );
    }

    #[test]
    fn uri_to_path_localhost_authority_accepted() {
        let uri = lsp_types::Uri::from_str("file://localhost/tmp/x").expect("parse");
        assert_eq!(
            uri_to_path(&uri).expect("uri_to_path"),
            PathBuf::from("/tmp/x")
        );
    }

    #[test]
    fn uri_to_path_rejects_http_scheme() {
        let uri = lsp_types::Uri::from_str("http://example.com/x").expect("parse");
        assert_eq!(uri_to_path(&uri), Err(UriError::NotFileScheme));
    }

    #[test]
    fn uri_to_path_rejects_relative_reference() {
        let uri = lsp_types::Uri::from_str("relative/path").expect("parse");
        assert_eq!(uri_to_path(&uri), Err(UriError::NotFileScheme));
    }

    #[test]
    fn uri_to_path_rejects_non_localhost_authority() {
        let uri = lsp_types::Uri::from_str("file://example.com/x").expect("parse");
        assert_eq!(
            uri_to_path(&uri),
            Err(UriError::BadAuthority("example.com".to_owned()))
        );
    }

    #[test]
    fn uri_to_path_rejects_percent_encoded_traversal_segment() {
        // "%2F" inside a single segment decodes to '/', which would silently
        // inject an extra path boundary — must be rejected, not merged.
        let uri = lsp_types::Uri::from_str("file:///tmp/etc%2Fpasswd").expect("parse");
        assert!(matches!(uri_to_path(&uri), Err(UriError::Decode(_))));
    }

    // ── Windows-shaped: string-level, runs on every OS ──────────────────────

    #[test]
    fn windows_drive_path_string_level_uri_and_back() {
        // Build the expected URI string for a synthetic "C:\Users\x.rs" path
        // directly, without depending on Path::is_absolute()'s host-OS
        // semantics — this must hold everywhere, not just on Windows CI.
        let uri = lsp_types::Uri::from_str("file:///C:/Users/x.rs").expect("parse");
        assert_eq!(
            uri_to_path(&uri).expect("uri_to_path"),
            PathBuf::from("C:\\Users\\x.rs")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn strip_verbatim_prefix_is_a_no_op_off_windows() {
        assert_eq!(strip_verbatim_prefix(r"\\?\C:\foo"), r"\\?\C:\foo");
    }

    // ── Windows-real: exercises Path::is_absolute() for real ───────────────

    #[cfg(windows)]
    #[test]
    fn windows_absolute_path_round_trips() {
        let path = Path::new(r"C:\Users\x.rs");
        assert_eq!(round_trip(path), path);
    }

    #[cfg(windows)]
    #[test]
    fn windows_verbatim_prefix_is_stripped() {
        let uri = path_to_uri(Path::new(r"\\?\C:\Users\x.rs")).expect("path_to_uri");
        assert_eq!(uri.as_str(), "file:///C:/Users/x.rs");
    }

    #[cfg(windows)]
    #[test]
    fn strip_verbatim_prefix_strips_but_not_unc() {
        assert_eq!(strip_verbatim_prefix(r"\\?\C:\foo"), r"C:\foo");
        assert_eq!(
            strip_verbatim_prefix(r"\\?\UNC\server\share"),
            r"\\?\UNC\server\share"
        );
    }
}
