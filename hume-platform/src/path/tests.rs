use std::path::PathBuf;

use super::*;

// ── expand_with ───────────────────────────────────────────────────────────

fn no_env(_: &str) -> Result<String, std::env::VarError> {
    Err(std::env::VarError::NotPresent)
}

fn home(h: &'static str) -> impl FnOnce() -> Option<PathBuf> {
    move || Some(PathBuf::from(h))
}

fn no_home() -> Option<PathBuf> {
    None
}

#[test]
fn expand_absolute_path_is_borrowed() {
    let result = expand_with("/abs/path", no_env, no_home);
    assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
    assert_eq!(result, "/abs/path");
}

#[test]
fn expand_relative_path_is_borrowed() {
    let result = expand_with("relative/path", no_env, no_home);
    assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
    assert_eq!(result, "relative/path");
}

#[test]
fn expand_tilde_alone() {
    let result = expand_with("~", no_env, home("/home/user"));
    assert_eq!(result, "/home/user");
}

#[test]
fn expand_tilde_slash() {
    let result = expand_with("~/foo", no_env, home("/home/user"));
    assert_eq!(result, "/home/user/foo");
}

#[test]
fn expand_tilde_user_form_unchanged() {
    let result = expand_with("~alice/foo", no_env, home("/home/user"));
    // `~alice` is not expanded (no getpwnam).
    assert_eq!(result, "~alice/foo");
}

#[test]
fn expand_tilde_mid_string_unchanged() {
    let result = expand_with("/foo~bar", no_env, no_home);
    assert_eq!(result, "/foo~bar");
}

#[test]
fn expand_tilde_home_unset_leaves_literal() {
    let result = expand_with("~/foo", no_env, no_home);
    assert_eq!(result, "~/foo");
}

// ── split_path_at_sep ─────────────────────────────────────────────────────

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

// ── is_safe_segment ───────────────────────────────────────────────────────

#[test]
fn is_safe_segment_empty_is_rejected() {
    assert!(!is_safe_segment(""));
}

#[test]
fn is_safe_segment_dot_is_rejected() {
    assert!(!is_safe_segment("."));
}

#[test]
fn is_safe_segment_dotdot_is_rejected() {
    assert!(!is_safe_segment(".."));
}

#[test]
fn is_safe_segment_forward_slash_is_rejected() {
    assert!(!is_safe_segment("a/b"));
}

#[test]
fn is_safe_segment_backslash_is_rejected() {
    assert!(!is_safe_segment(r"a\b"));
}

#[test]
fn is_safe_segment_nul_is_rejected() {
    assert!(!is_safe_segment("a\0b"));
}

#[test]
fn is_safe_segment_quote_is_rejected() {
    assert!(!is_safe_segment("a\"b"));
}

#[test]
fn is_safe_segment_colon_is_rejected() {
    // "c:evil" would make PathBuf::push treat the segment as a Windows
    // drive-relative root, escaping the sandboxed base directory.
    assert!(!is_safe_segment("c:evil"));
    assert!(!is_safe_segment("a:b"));
}

#[test]
fn is_safe_segment_version_string_is_valid() {
    // Dots within a segment are fine (e.g. "v1.2.3").
    assert!(is_safe_segment("v1.2.3"));
}

#[test]
fn is_safe_segment_plain_name_is_valid() {
    assert!(is_safe_segment("helix-surround"));
}

// ── relative_to ───────────────────────────────────────────────────────────

#[test]
fn relative_to_strips_the_base() {
    let path = AbsolutePath::rooted(Path::new("/a/b").join("sub").join("f.rs"));
    assert_eq!(
        relative_to(&path, &AbsolutePath::rooted("/a/b")),
        Some(Path::new("sub").join("f.rs"))
    );
}

#[test]
fn relative_to_is_none_outside_the_base() {
    assert_eq!(
        relative_to(
            &AbsolutePath::rooted("/x/f.rs"),
            &AbsolutePath::rooted("/a/b")
        ),
        None
    );
}

#[test]
fn relative_to_matches_whole_components_only() {
    assert_eq!(
        relative_to(
            &AbsolutePath::rooted("/a/bc/f.rs"),
            &AbsolutePath::rooted("/a/b")
        ),
        None
    );
}

#[test]
fn relative_to_the_base_itself_is_empty() {
    assert_eq!(
        relative_to(&AbsolutePath::rooted("/a/b"), &AbsolutePath::rooted("/a/b")),
        Some(PathBuf::new())
    );
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
