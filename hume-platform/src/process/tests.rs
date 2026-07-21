use super::*;

#[test]
fn run_inline_output_missing_binary_is_io_error() {
    assert!(run_inline_output("definitely-not-a-real-binary-xyz", &[], None).is_err());
}

// ── sha256 output parsing (pure, all platforms) ──────────────────────────

#[test]
fn parse_unix_sha256_output_takes_first_token() {
    let stdout =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  fixture.bin\n";
    assert_eq!(
        parse_unix_sha256_output(stdout).as_deref(),
        Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );
}

#[test]
fn parse_unix_sha256_output_lowercases() {
    let stdout = "ABCD1234  fixture.bin\n";
    assert_eq!(
        parse_unix_sha256_output(stdout).as_deref(),
        Some("abcd1234")
    );
}

#[test]
fn parse_unix_sha256_output_empty_is_none() {
    assert_eq!(parse_unix_sha256_output(""), None);
}

#[test]
fn parse_certutil_sha256_output_takes_second_line() {
    let stdout = "SHA256 hash of file fixture.bin:\r\nab 12 cd 34\r\nCertUtil: -hashfile command completed successfully.\r\n";
    assert_eq!(
        parse_certutil_sha256_output(stdout).as_deref(),
        Some("ab12cd34")
    );
}

#[test]
fn parse_certutil_sha256_output_missing_hex_line_is_none() {
    assert_eq!(
        parse_certutil_sha256_output("SHA256 hash of file f:\r\n"),
        None
    );
}

#[test]
fn parse_certutil_sha256_output_empty_is_none() {
    assert_eq!(parse_certutil_sha256_output(""), None);
}

// ── sha256_file (shells out to the real platform tool) ───────────────────

#[test]
fn sha256_file_matches_precomputed_digest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fixture.bin");
    std::fs::write(&path, b"hume").expect("write fixture");
    // `printf hume | shasum -a 256` / `sha256sum` precomputed digest.
    let expected = "604f73953b84e48e552fea0b7fed0d938b038b5b1b18f7c10f5bb640ae5e9c40";
    let got = sha256_file(&path).expect("sha256_file");
    assert_eq!(got, expected);
}

#[test]
fn sha256_file_missing_source_is_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("does-not-exist.bin");
    assert!(sha256_file(&path).is_err());
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
