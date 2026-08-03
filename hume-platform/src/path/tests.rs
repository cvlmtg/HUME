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

// ── normalize_lexical ─────────────────────────────────────────────────────

#[test]
fn normalize_lexical_removes_cur_dir() {
    assert_eq!(normalize_lexical(Path::new("a/./b")), PathBuf::from("a/b"));
}

#[test]
fn normalize_lexical_pops_parent_dir() {
    assert_eq!(
        normalize_lexical(Path::new("a/b/../c")),
        PathBuf::from("a/c")
    );
}

#[test]
fn normalize_lexical_pop_on_empty_is_safe() {
    // Leading ".." when the output is empty: pop() on an empty PathBuf is a
    // no-op, so the ".." is silently discarded and only "a" survives.
    assert_eq!(normalize_lexical(Path::new("../a")), PathBuf::from("a"));
}

#[test]
fn normalize_lexical_normal_path_unchanged() {
    assert_eq!(
        normalize_lexical(Path::new("a/b/c")),
        PathBuf::from("a/b/c")
    );
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

// ── absolute_unresolved ───────────────────────────────────────────────────

#[test]
fn absolute_unresolved_relative_joins_cwd() {
    let cwd = PathBuf::from("/home/user/projects");
    let result = absolute_unresolved(std::path::Path::new("foo/bar.txt"), &cwd);
    assert_eq!(result, PathBuf::from("/home/user/projects/foo/bar.txt"));
}

#[test]
fn absolute_unresolved_absolute_is_unchanged() {
    let cwd = PathBuf::from("/home/user/projects");
    let result = absolute_unresolved(std::path::Path::new("/tmp/foo.txt"), &cwd);
    assert_eq!(result, PathBuf::from("/tmp/foo.txt"));
}

#[test]
fn absolute_unresolved_collapses_dots() {
    let cwd = PathBuf::from("/home/user");
    // `./foo/../bar` → `/home/user/bar`
    let result = absolute_unresolved(std::path::Path::new("./foo/../bar.txt"), &cwd);
    assert_eq!(result, PathBuf::from("/home/user/bar.txt"));
}

#[test]
fn absolute_unresolved_does_not_canonicalize_symlinks() {
    // We can't make a symlink in a pure unit test, but we can verify that
    // a directory name that looks like it could be a symlink (e.g. "link")
    // is passed through unchanged — no fs access occurs.
    let cwd = PathBuf::from("/real/path");
    let result = absolute_unresolved(std::path::Path::new("../symlink-dir/file.txt"), &cwd);
    // `../` from `/real/path` → `/real` (lexical pop), then `symlink-dir/file.txt`
    assert_eq!(result, PathBuf::from("/real/symlink-dir/file.txt"));
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
