//! Unix-only URI conversion tests, gated once at the `mod unix;`
//! declaration in the parent.

use super::*;

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
fn round_trip_path_with_colon() {
    // ':' is a legal Unix filename byte and pchar-legal in a URI path
    // segment — must round-trip bare, not as "%3A".
    let path = Path::new("/tmp/weird:name.rs");
    assert_eq!(round_trip(path), path);
}

#[test]
fn round_trip_path_with_a_literal_backslash_on_unix() {
    // A literal '\' is an ordinary Unix filename byte, not a separator —
    // must round-trip as one path component, not get silently split.
    let path = Path::new(r"/tmp/weird\name.rs");
    assert_eq!(round_trip(path), path);
}

#[test]
fn path_to_uri_rejects_non_utf8_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(OsStr::from_bytes(b"/tmp/\xffbad"));
    assert_eq!(path_to_uri(path), Err(UriError::NotUtf8));
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

#[test]
fn path_to_uri_leaves_colon_unescaped() {
    let uri = path_to_uri(Path::new("/tmp/weird:name.rs")).expect("path_to_uri");
    assert_eq!(uri.as_str(), "file:///tmp/weird:name.rs");
}

#[test]
fn drive_letter_shaped_segment_is_a_literal_directory_name_on_unix() {
    // "C:" is only a drive letter on Windows — on Unix it's simply a
    // directory literally named "C:", decoded as one segment among
    // others, never hoisted into a Windows-style "C:\..." path.
    let uri = lsp_types::Uri::from_str("file:///C:/Users/x.rs").expect("parse");
    assert_eq!(
        uri_to_path(&uri).expect("uri_to_path"),
        PathBuf::from("/C:/Users/x.rs")
    );
}

#[test]
fn decoded_backslash_in_a_segment_is_accepted_on_unix() {
    // A literal '\' is a normal Unix filename byte — percent-encoded on
    // the way out (see round_trip_path_with_a_literal_backslash_on_unix)
    // and must be accepted, not rejected, decoding back in.
    let uri = lsp_types::Uri::from_str("file:///tmp/weird%5Cname.rs").expect("parse");
    assert_eq!(
        uri_to_path(&uri).expect("uri_to_path"),
        PathBuf::from("/tmp/weird\\name.rs")
    );
}

// Windows: a non-localhost authority is read as a UNC server name
// instead of rejected — see
// uri_to_path_reconstructs_a_unc_path_from_a_non_localhost_authority.
#[test]
fn uri_to_path_rejects_non_localhost_authority() {
    let uri = lsp_types::Uri::from_str("file://example.com/x").expect("parse");
    assert_eq!(
        uri_to_path(&uri),
        Err(UriError::BadAuthority("example.com".to_owned()))
    );
}

// ── Windows-shaped: string-level, runs on every OS ──────────────────────

#[test]
fn a_windows_shaped_verbatim_prefix_round_trips_unmodified_off_windows() {
    // `\\?\` is meaningful only to the Windows path API — off Windows it's
    // just ordinary filename bytes, so verbatim-prefix stripping must never
    // fire here.
    let path = Path::new(r"/tmp/\\?\C:\foo");
    assert_eq!(round_trip(path), path);
}
