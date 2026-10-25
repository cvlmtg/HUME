//! Unix-only tests, gated once at the `mod unix;` declaration in the parent.

use std::os::unix::fs::symlink;

use super::AbsolutePath;

#[test]
fn resolve_follows_a_dangling_symlink_to_the_missing_path_it_names() {
    let dir = tempfile::tempdir().unwrap();
    symlink(dir.path().join("target.txt"), dir.path().join("link")).unwrap();
    let canonical = AbsolutePath::canonicalize(dir.path()).unwrap();
    assert_eq!(
        AbsolutePath::rooted(dir.path().join("link"))
            .resolve()
            .unwrap()
            .as_path(),
        canonical.join("target.txt")
    );
}

#[test]
fn resolve_follows_a_chain_of_dangling_symlinks_to_its_end() {
    let dir = tempfile::tempdir().unwrap();
    symlink("middle", dir.path().join("first")).unwrap();
    symlink("sub/end.txt", dir.path().join("middle")).unwrap();
    let canonical = AbsolutePath::canonicalize(dir.path()).unwrap();
    assert_eq!(
        AbsolutePath::rooted(dir.path().join("first"))
            .resolve()
            .unwrap()
            .as_path(),
        canonical.join("sub").join("end.txt")
    );
}

#[test]
fn resolve_fails_for_a_symlink_cycle() {
    let dir = tempfile::tempdir().unwrap();
    symlink("b", dir.path().join("a")).unwrap();
    symlink("a", dir.path().join("b")).unwrap();
    assert!(
        AbsolutePath::rooted(dir.path().join("a"))
            .resolve()
            .is_err()
    );
}

#[test]
fn resolve_fails_for_a_dot_dot_after_a_missing_directory() {
    let dir = tempfile::tempdir().unwrap();
    symlink("missing/../f.txt", dir.path().join("link")).unwrap();
    let err = AbsolutePath::rooted(dir.path().join("link"))
        .resolve()
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}
