//! Windows-only URI conversion tests, gated once at the `mod windows;`
//! declaration in the parent.

use super::*;

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
fn decoded_backslash_in_a_segment_is_rejected_on_windows() {
    // On Windows a decoded '\' inside one segment would be ambiguous
    // with a real separator once the result reaches PathBuf — reject
    // it, same as a decoded '/'.
    let uri = lsp_types::Uri::from_str("file:///tmp/weird%5Cname.rs").expect("parse");
    assert!(matches!(uri_to_path(&uri), Err(UriError::Decode(_))));
}

// ── Windows-real: exercises Path::is_absolute() for real ───────────────

#[test]
fn windows_drive_path_string_level_uri_and_back() {
    let uri = lsp_types::Uri::from_str("file:///C:/Users/x.rs").expect("parse");
    assert_eq!(
        uri_to_path(&uri).expect("uri_to_path"),
        PathBuf::from("C:\\Users\\x.rs")
    );
}

#[test]
fn windows_absolute_path_round_trips() {
    let path = Path::new(r"C:\Users\x.rs");
    assert_eq!(round_trip(path), path);
}

#[test]
fn windows_verbatim_prefix_is_stripped() {
    let uri = path_to_uri(Path::new(r"\\?\C:\Users\x.rs")).expect("path_to_uri");
    assert_eq!(uri.as_str(), "file:///C:/Users/x.rs");
}

#[test]
fn strip_verbatim_prefix_strips_but_not_unc() {
    assert_eq!(strip_verbatim_prefix(r"\\?\C:\foo"), r"C:\foo");
    assert_eq!(
        strip_verbatim_prefix(r"\\?\UNC\server\share"),
        r"\\?\UNC\server\share"
    );
}

#[test]
fn windows_unc_path_round_trips() {
    let path = Path::new(r"\\myserver\share\x.rs");
    assert_eq!(round_trip(path), path);
    let uri = path_to_uri(path).expect("path_to_uri");
    assert_eq!(uri.as_str(), "file://myserver/share/x.rs");
}

#[test]
fn windows_unc_verbatim_prefix_round_trips() {
    let path = Path::new(r"\\?\UNC\myserver\share\x.rs");
    let uri = path_to_uri(path).expect("path_to_uri");
    assert_eq!(uri.as_str(), "file://myserver/share/x.rs");
    assert_eq!(
        uri_to_path(&uri).expect("uri_to_path"),
        PathBuf::from(r"\\myserver\share\x.rs")
    );
}

#[test]
fn uri_to_path_reconstructs_a_unc_path_from_a_non_localhost_authority() {
    let uri = lsp_types::Uri::from_str("file://myserver/share/x.rs").expect("parse");
    assert_eq!(
        uri_to_path(&uri).expect("uri_to_path"),
        PathBuf::from(r"\\myserver\share\x.rs")
    );
}
