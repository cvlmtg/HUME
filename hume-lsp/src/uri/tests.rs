use super::*;

fn round_trip(path: &Path) -> PathBuf {
    let uri = path_to_uri(path).expect("path_to_uri");
    uri_to_path(&uri).expect("uri_to_path")
}

// ── path_to_uri: relative rejected ──────────────────────────────────────

#[test]
fn path_to_uri_rejects_relative_path() {
    assert_eq!(
        path_to_uri(Path::new("relative/path.rs")),
        Err(UriError::NotAbsolute)
    );
}

// ── path_to_file_url: host in the authority ─────────────────────────────

#[cfg(not(windows))]
#[test]
fn path_to_file_url_puts_the_host_in_the_authority() {
    assert_eq!(
        path_to_file_url(Path::new("/tmp/a b"), "box").as_deref(),
        Ok("file://box/tmp/a%20b")
    );
}

#[cfg(not(windows))]
#[test]
fn path_to_file_url_percent_encodes_the_host() {
    assert_eq!(
        path_to_file_url(Path::new("/tmp"), "my box").as_deref(),
        Ok("file://my%20box/tmp")
    );
}

#[test]
fn path_to_file_url_rejects_relative_path() {
    assert_eq!(
        path_to_file_url(Path::new("rel/dir"), "box"),
        Err(UriError::NotAbsolute)
    );
}

#[cfg(windows)]
#[test]
fn path_to_file_url_puts_the_host_before_a_drive_letter_path() {
    assert_eq!(
        path_to_file_url(Path::new(r"D:\test"), "box").as_deref(),
        Ok("file://box/D:/test")
    );
}

#[cfg(windows)]
#[test]
fn path_to_file_url_keeps_a_unc_server_as_the_authority() {
    assert_eq!(
        path_to_file_url(Path::new(r"\\srv\share\x"), "box").as_deref(),
        Ok("file://srv/share/x")
    );
}

// ── uri_to_display_string: what a drawer row shows ──────────────────────

#[test]
fn uri_to_display_string_drops_the_slash_before_a_drive_letter_on_every_platform() {
    // A server on Windows can report a location to an editor running
    // anywhere. `uri_to_path` keeps the leading slash off Windows because
    // `/C:/…` is the literal path there, but a drawer row is text, and
    // `C:/Users/x/main.rs` is how that location is named.
    let uri = lsp_types::Uri::from_str("file:///C:/Users/x/main.rs").expect("parse");
    assert_eq!(
        uri_to_display_string(&uri).expect("uri_to_display_string"),
        "C:/Users/x/main.rs"
    );
}

#[test]
fn uri_to_display_string_percent_decodes() {
    let uri = lsp_types::Uri::from_str("file:///tmp/a%20name.rs").expect("parse");
    assert_eq!(
        uri_to_display_string(&uri).expect("uri_to_display_string"),
        "/tmp/a name.rs"
    );
}

#[test]
fn uri_to_display_string_leaves_an_ordinary_absolute_path_alone() {
    // Only a real drive-letter segment loses the slash: a one-letter
    // directory must not.
    let uri = lsp_types::Uri::from_str("file:///c/src/main.rs").expect("parse");
    assert_eq!(
        uri_to_display_string(&uri).expect("uri_to_display_string"),
        "/c/src/main.rs"
    );
}

// ── uri_to_path: inbound parsing ────────────────────────────────────────

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
fn uri_to_path_rejects_percent_encoded_traversal_segment() {
    // "%2F" inside a single segment decodes to '/', which would silently
    // inject an extra path boundary. It must be rejected, not merged.
    let uri = lsp_types::Uri::from_str("file:///tmp/etc%2Fpasswd").expect("parse");
    assert!(matches!(uri_to_path(&uri), Err(UriError::Decode(_))));
}

#[test]
fn uri_to_path_rejects_dot_dot_segment() {
    let uri = lsp_types::Uri::from_str("file:///tmp/../etc/passwd").expect("parse");
    assert!(matches!(uri_to_path(&uri), Err(UriError::Decode(_))));
}

#[test]
fn uri_to_path_rejects_percent_encoded_dot_dot_segment() {
    // "%2E%2E" decodes to "..", same traversal hazard as the literal form:
    // must be caught after decoding, not before.
    let uri = lsp_types::Uri::from_str("file:///tmp/%2E%2E/etc/passwd").expect("parse");
    assert!(matches!(uri_to_path(&uri), Err(UriError::Decode(_))));
}

#[test]
fn uri_to_path_rejects_single_dot_segment() {
    let uri = lsp_types::Uri::from_str("file:///tmp/./passwd").expect("parse");
    assert!(matches!(uri_to_path(&uri), Err(UriError::Decode(_))));
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
