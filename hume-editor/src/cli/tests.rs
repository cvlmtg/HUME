use super::*;
use crate::editor::tests::safe_tempdir;

// `tmp.path().join(name)` below is guaranteed absent from disk — `tmp` is a
// freshly created, otherwise empty tempdir.

#[test]
fn line_only_suffix() {
    let tmp = safe_tempdir();
    let arg = tmp.path().join("foo.rs:12");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, tmp.path().join("foo.rs"));
    assert_eq!(
        parsed.pos,
        Some(CliPosition {
            line: 12,
            grapheme_col: 1
        })
    );
}

#[test]
fn line_and_column_suffix() {
    let tmp = safe_tempdir();
    let arg = tmp.path().join("foo.rs:12:24");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, tmp.path().join("foo.rs"));
    assert_eq!(
        parsed.pos,
        Some(CliPosition {
            line: 12,
            grapheme_col: 24
        })
    );
}

#[test]
fn trailing_colon_is_tolerated() {
    let tmp = safe_tempdir();
    let arg = tmp.path().join("foo.rs:12:");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, tmp.path().join("foo.rs"));
    assert_eq!(
        parsed.pos,
        Some(CliPosition {
            line: 12,
            grapheme_col: 1
        })
    );
}

#[test]
fn no_suffix_is_a_plain_path() {
    let tmp = safe_tempdir();
    let arg = tmp.path().join("foo.rs");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, arg);
    assert_eq!(parsed.pos, None);
}

#[test]
fn non_digit_suffix_is_a_literal_path() {
    let tmp = safe_tempdir();
    let arg = tmp.path().join("foo.rs:abc");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, arg);
    assert_eq!(parsed.pos, None);
}

#[test]
fn bare_colon_number_with_no_path_is_literal() {
    let tmp = safe_tempdir();
    // `tmp.path().join(":12")` is `/tmp/…/:12` — rsplit_once(':') leaves a
    // remainder ending in the path separator (`/tmp/…/`), which
    // `split_trailing_number`'s `rest.ends_with(is_separator)` check rejects
    // as naming no file. A relative `":12"` (remainder truly empty, not just
    // separator-terminated) exercises the check's other arm — see
    // `relative_bare_colon_number_is_literal` below.
    let arg = tmp.path().join(":12");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, arg);
    assert_eq!(parsed.pos, None);
}

#[test]
fn relative_bare_colon_number_is_literal() {
    // A *relative* ":12" (no directory component at all, unlike
    // `tmp.path().join(":12")` above) is the shape that actually leaves
    // `split_trailing_number`'s `rsplit_once(':')` remainder empty —
    // exercising `rest.is_empty()` rather than `rest.ends_with(is_separator)`.
    let arg = PathBuf::from(":12");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, arg);
    assert_eq!(parsed.pos, None);
}

#[test]
fn line_zero_is_rejected() {
    let tmp = safe_tempdir();
    let arg = tmp.path().join("foo.rs:0");
    let err = parse_file_arg(&arg).unwrap_err();
    assert!(err.contains(LINE_NUMBERS_START_AT_1), "got: {err}");
    assert!(
        err.contains(&arg.display().to_string()),
        "error must name the offending argument, got: {err}"
    );
}

#[test]
fn column_zero_is_rejected() {
    let tmp = safe_tempdir();
    let arg = tmp.path().join("foo.rs:12:0");
    let err = parse_file_arg(&arg).unwrap_err();
    // A valid line paired with a bad column must not blame the line — the
    // two 1-based contracts get distinct error text.
    assert!(err.contains(GRAPHEME_COL_NUMBERS_START_AT_1), "got: {err}");
    assert!(
        !err.contains(LINE_NUMBERS_START_AT_1),
        "line was valid — must not be blamed, got: {err}"
    );
    assert!(
        err.contains(&arg.display().to_string()),
        "error must name the offending argument, got: {err}"
    );
}

#[cfg(unix)]
#[test]
fn a_real_file_named_with_a_colon_opens_literally() {
    let tmp = safe_tempdir();
    let path = tmp.path().join("weird:12");
    std::fs::write(&path, "hello\n").unwrap();
    let parsed = parse_file_arg(&path).unwrap();
    assert_eq!(parsed.path, path);
    assert_eq!(
        parsed.pos, None,
        "an existing literal path must never be split"
    );
}

#[cfg(unix)]
#[test]
fn a_broken_symlink_named_with_a_colon_opens_literally() {
    // `symlink_metadata` (not `.exists()`, which follows symlinks and would
    // report this one absent) is the deliberate disambiguation probe here —
    // a dangling symlink still names a path the user chose on disk, so it
    // must win over splitting the same as a real file does.
    let tmp = safe_tempdir();
    let path = tmp.path().join("weird:12");
    std::os::unix::fs::symlink(tmp.path().join("does-not-exist-target"), &path).unwrap();
    let parsed = parse_file_arg(&path).unwrap();
    assert_eq!(parsed.path, path);
    assert_eq!(
        parsed.pos, None,
        "a broken symlink still counts as the user's chosen path — must never be split"
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_path_is_always_literal() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let raw = OsStr::from_bytes(b"weird-\xff-path:12");
    let path = PathBuf::from(raw);
    let parsed = parse_file_arg(&path).unwrap();
    assert_eq!(parsed.path, path);
    assert_eq!(parsed.pos, None);
}

#[cfg(windows)]
#[test]
fn bare_drive_letter_is_not_split_as_a_position() {
    let arg = PathBuf::from("C:12");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, arg);
    assert_eq!(parsed.pos, None);
}

#[cfg(windows)]
#[test]
fn drive_absolute_path_with_line_suffix_splits_normally() {
    let arg = PathBuf::from(r"C:\src\a.rs:12");
    let parsed = parse_file_arg(&arg).unwrap();
    assert_eq!(parsed.path, PathBuf::from(r"C:\src\a.rs"));
    assert_eq!(
        parsed.pos,
        Some(CliPosition {
            line: 12,
            grapheme_col: 1
        })
    );
}
