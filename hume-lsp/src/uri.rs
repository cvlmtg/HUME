//! Lossless, canonical path ↔ `file://` URI conversion.
//!
//! Outgoing URIs always come from the canonical `Buffer.path` (the SSOT, on
//! the `hume-editor` side); incoming URIs are converted to paths here and
//! canonicalized by the caller before buffer lookup; this module never
//! guesses a path on error, it only ever returns `Err`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use hume_platform::file_url::{FileUrlError, normalize_separators, path_to_file_url};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// The URI's scheme is not `file` (or has no scheme at all).
    NotFileScheme,
    /// [`path_to_uri`]'s input path was not absolute.
    NotAbsolute,
    /// [`path_to_uri`]'s input path is not valid UTF-8, never silently
    /// mangled via a lossy conversion.
    NotUtf8,
    /// The URI's authority is present and is neither empty nor `localhost`.
    /// Windows: any other host is instead read as a UNC server name; see
    /// [`uri_to_path`].
    BadAuthority(String),
    /// The URI's path failed to percent-decode, or a decoded segment is a
    /// traversal component (`.` or `..`) or contains a `/` (always) or
    /// (Windows only, where `\` is also a separator) a `\` disguising an
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

impl From<FileUrlError> for UriError {
    fn from(e: FileUrlError) -> Self {
        match e {
            FileUrlError::NotAbsolute => UriError::NotAbsolute,
            FileUrlError::NotUtf8 => UriError::NotUtf8,
        }
    }
}

/// Absolute path → `file://` URI. See [`path_to_file_url`] for the encoding.
///
/// # Errors
/// [`UriError::NotAbsolute`] if `path` is relative, never joined against a
/// cwd; the caller owns canonicalization. [`UriError::NotUtf8`] if `path`
/// is not valid UTF-8, never silently mangled via a lossy conversion.
pub fn path_to_uri(path: &Path) -> Result<lsp_types::Uri, UriError> {
    let url = path_to_file_url(path, "")?;
    Ok(lsp_types::Uri::from_str(&url)
        .expect("percent-encoded file URI is always syntactically valid"))
}

/// `file://` URI → absolute path. Accepts empty and `localhost` authority;
/// rejects other schemes/authorities loudly, never a guessed path. Windows:
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

    // UNC form: "file://server/share/foo": reconstruct "\\server\share\foo"
    // rather than falling through to the leading-slash form below.
    #[cfg(windows)]
    if let Some(host) = unc_host {
        let mut path = format!(r"\\{host}\");
        path.push_str(&segments.join("\\"));
        return Ok(PathBuf::from(path));
    }

    // Windows drive-letter form: "file:///C:/foo" (or the colon escaped as
    // "file:///c%3A/foo") decodes to a first segment "C:". Join without a
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
/// `Ok(Some(host))` for any other authority, read as a UNC server name.
/// Elsewhere, any other authority is rejected outright: UNC has no meaning
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

/// `true` if `segment` is a single ASCII letter followed by `:` (e.g. `"C:"`):
/// the decoded shape of a Windows drive-letter path segment.
fn is_drive_letter_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// `file://` URI → the string form to *show* a user, as
/// [`uri_to_path`] decodes it but with a Windows drive-letter path rendered
/// `C:/foo` on every platform, not just Windows.
///
/// Separate from [`uri_to_path`] because the two answer different questions.
/// `uri_to_path` produces a path to *open*: off Windows, `file:///C:/foo`
/// names the literal directory `/C:`, and dropping that leading slash would
/// turn an absolute path into a relative one resolved against the cwd. A
/// drawer row naming a location on another machine has no such constraint
/// (it is text), and showing `/C:/Users/x/main.rs` for a path the server
/// calls `C:/Users/x/main.rs` just looks wrong.
///
/// Errors exactly as [`uri_to_path`] does.
pub fn uri_to_display_string(uri: &lsp_types::Uri) -> Result<String, UriError> {
    let path = uri_to_path(uri)?;
    let display = normalize_separators(&path.display().to_string());
    Ok(match display.strip_prefix('/') {
        Some(rest) if rest.split('/').next().is_some_and(is_drive_letter_segment) => {
            rest.to_string()
        }
        _ => display,
    })
}

#[cfg(test)]
mod tests;
