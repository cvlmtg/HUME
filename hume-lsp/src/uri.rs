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
    /// [`path_to_uri`]'s input path is not valid UTF-8 — never silently
    /// mangled via a lossy conversion.
    NotUtf8,
    /// The URI's authority is present and is neither empty nor `localhost`.
    /// Windows: any other host is instead read as a UNC server name — see
    /// [`uri_to_path`].
    BadAuthority(String),
    /// The URI's path failed to percent-decode, or a decoded segment is a
    /// traversal component (`.` or `..`) or contains a `/` (always) or —
    /// Windows only, where `\` is also a separator — a `\` disguising an
    /// extra path boundary (rejected defensively, see [`uri_to_path`]).
    Decode(String),
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UriError::NotFileScheme => write!(f, "URI is not a file:// URI"),
            UriError::NotAbsolute => write!(f, "path is not absolute"),
            UriError::NotUtf8 => write!(f, "path is not valid UTF-8"),
            UriError::BadAuthority(host) => write!(f, "unsupported URI authority: {host:?}"),
            UriError::Decode(msg) => write!(f, "failed to decode URI path: {msg}"),
        }
    }
}

impl std::error::Error for UriError {}

/// Absolute path → `file://` URI. Percent-encodes everything but unreserved
/// chars (`A-Za-z0-9-._~`), `/`, and `:` (pchar-legal, left bare so a drive
/// letter reads `C:` not `C%3A`). Windows: any `\\?\` verbatim prefix is
/// stripped first; a UNC path (`\\server\share\…` or
/// `\\?\UNC\server\share\…`) emits `server` as the URI authority
/// (`file://server/share/…`) instead of folding it into the path;
/// otherwise backslashes become `/` and a drive letter gets a synthetic
/// leading `/` so the result reads `file:///C:/…`.
///
/// # Errors
/// [`UriError::NotAbsolute`] if `path` is relative — never joined against a
/// cwd; the caller owns canonicalization. [`UriError::NotUtf8`] if `path`
/// is not valid UTF-8 — never silently mangled via a lossy conversion.
pub fn path_to_uri(path: &Path) -> Result<lsp_types::Uri, UriError> {
    if !path.is_absolute() {
        return Err(UriError::NotAbsolute);
    }

    let path_str = path.to_str().ok_or(UriError::NotUtf8)?;
    let stripped = strip_verbatim_prefix(path_str);

    #[cfg(windows)]
    if let Some((host, rest)) = unc_host_and_rest(stripped) {
        let with_leading_slash = ensure_leading_slash(normalize_separators(rest));
        let uri_str = format!(
            "file://{}{}",
            percent_encode_path(host),
            percent_encode_path(&with_leading_slash)
        );
        return Ok(lsp_types::Uri::from_str(&uri_str)
            .expect("percent-encoded file URI is always syntactically valid"));
    }

    let with_leading_slash = ensure_leading_slash(normalize_separators(stripped));
    let encoded = percent_encode_path(&with_leading_slash);
    let uri_str = format!("file://{encoded}");
    Ok(lsp_types::Uri::from_str(&uri_str)
        .expect("percent-encoded file URI is always syntactically valid"))
}

/// Backslash → `/`, so a Windows path reads as a URI path. A no-op on
/// Unix, where `\` is an ordinary, legal filename byte that must round-trip
/// untouched (percent-encoded on the way out, accepted verbatim back in —
/// see [`uri_to_path`]).
#[cfg(windows)]
fn normalize_separators(s: &str) -> String {
    s.chars().map(|c| if c == '\\' { '/' } else { c }).collect()
}

#[cfg(not(windows))]
fn normalize_separators(s: &str) -> String {
    s.to_owned()
}

/// Prefixes `s` with `/` unless it already starts with one — a URI path
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
/// `\\?\`-verbatim path (already stripped by [`strip_verbatim_prefix`]
/// before this runs).
#[cfg(windows)]
fn unc_host_and_rest(s: &str) -> Option<(&str, &str)> {
    let rest = s
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| s.strip_prefix(r"\\"))?;
    rest.split_once('\\')
}

/// `file://` URI → absolute path. Accepts empty and `localhost` authority;
/// rejects other schemes/authorities loudly — never a guessed path. Windows:
/// any other authority is read as a UNC server name instead of rejected.
///
/// # Errors
/// [`UriError::NotFileScheme`] for a non-`file` (or schemeless) URI,
/// [`UriError::BadAuthority`] for an authority that isn't empty, `localhost`,
/// or (Windows only) a UNC server name, [`UriError::Decode`] for a malformed
/// or traversal-hazardous path.
pub fn uri_to_path(uri: &lsp_types::Uri) -> Result<PathBuf, UriError> {
    let scheme = uri.scheme().ok_or(UriError::NotFileScheme)?;
    if !scheme.eq_lowercase("file") {
        return Err(UriError::NotFileScheme);
    }

    #[cfg(windows)]
    let unc_host = resolve_authority(uri)?;
    #[cfg(not(windows))]
    resolve_authority(uri)?;

    let mut segments = Vec::new();
    for raw_segment in uri.path().as_estr().split('/') {
        if raw_segment.as_str().is_empty() {
            continue; // the leading '/' of an absolute path produces one
        }
        let decoded = raw_segment
            .decode()
            .into_string()
            .map_err(|e| UriError::Decode(e.to_string()))?;
        if decoded == "." || decoded == ".." {
            return Err(UriError::Decode(format!(
                "segment {decoded:?} is a path traversal component"
            )));
        }
        if decoded.contains('/') {
            return Err(UriError::Decode(format!(
                "segment {decoded:?} decodes to contain a path separator"
            )));
        }
        #[cfg(windows)]
        if decoded.contains('\\') {
            return Err(UriError::Decode(format!(
                "segment {decoded:?} decodes to contain a path separator"
            )));
        }
        segments.push(decoded.into_owned());
    }

    // UNC form: "file://server/share/foo" — reconstruct "\\server\share\foo"
    // rather than falling through to the leading-slash form below.
    #[cfg(windows)]
    if let Some(host) = unc_host {
        let mut path = format!(r"\\{host}\");
        path.push_str(&segments.join("\\"));
        return Ok(PathBuf::from(path));
    }

    // Windows drive-letter form: "file:///C:/foo" (or the colon escaped as
    // "file:///c%3A/foo") decodes to a first segment "C:" — join without a
    // leading separator so the result reads "C:\foo", not "\C:\foo".
    #[cfg(windows)]
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

/// `Ok(None)` for no authority, or an empty/`localhost` one. Windows:
/// `Ok(Some(host))` for any other authority — read as a UNC server name.
/// Elsewhere, any other authority is rejected outright — UNC has no meaning
/// off Windows.
fn resolve_authority(uri: &lsp_types::Uri) -> Result<Option<String>, UriError> {
    let Some(authority) = uri.authority() else {
        return Ok(None);
    };
    let host = authority.host().as_str();
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return Ok(None);
    }
    #[cfg(windows)]
    {
        Ok(Some(host.to_owned()))
    }
    #[cfg(not(windows))]
    {
        Err(UriError::BadAuthority(host.to_owned()))
    }
}

/// `true` if `segment` is a single ASCII letter followed by `:` (e.g. `"C:"`)
/// — the decoded shape of a Windows drive-letter path segment.
#[cfg(windows)]
fn is_drive_letter_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// `true` for RFC 3986 unreserved characters.
fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Percent-encode every byte except unreserved chars, `/`, and `:` — the
/// only bytes [`path_to_uri`] leaves unencoded. `:` is pchar-legal per
/// RFC 3986 (not "unreserved", but still fine bare in a path segment), left
/// bare so a Windows drive letter reads `C:` rather than `C%3A`. Operates
/// byte-wise (not char-wise) so multi-byte UTF-8 sequences encode correctly
/// — each of their bytes is non-unreserved and gets its own `%XX`.
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
mod tests;
