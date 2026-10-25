use std::path::Path;

use super::AbsolutePath;

#[test]
fn from_path_joins_a_relative_path_onto_cwd_and_collapses_dots() {
    let cwd = AbsolutePath::rooted("/home/u");
    let typed = AbsolutePath::from_path(Path::new("./a/./b.txt"), &cwd).unwrap();
    assert_eq!(typed, AbsolutePath::rooted("/home/u/a/b.txt"));
}

#[test]
fn from_path_keeps_an_absolute_path() {
    let path = AbsolutePath::rooted("/srv/data/f.txt");
    let typed = AbsolutePath::from_path(&path, &AbsolutePath::rooted("/home/u")).unwrap();
    assert_eq!(typed, path);
}

#[test]
fn from_typed_expands_a_leading_tilde() {
    let home = crate::dirs::home_dir().expect("a home directory");
    let cwd = AbsolutePath::rooted("/elsewhere");
    let typed = AbsolutePath::from_typed("~/notes.md", &cwd).unwrap();
    assert_eq!(typed.as_path(), home.join("notes.md"));
}

#[test]
fn from_path_keeps_a_leading_tilde() {
    let cwd = AbsolutePath::rooted("/elsewhere");
    let given = AbsolutePath::from_path(Path::new("~/notes.md"), &cwd).unwrap();
    assert_eq!(given, AbsolutePath::rooted("/elsewhere/~/notes.md"));
}

#[cfg(windows)]
#[test]
fn from_path_rejects_a_drive_relative_path() {
    let cwd = AbsolutePath::rooted(r"C:\home\u");
    let err = AbsolutePath::from_path(Path::new("D:b.txt"), &cwd).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn current_dir_is_the_process_working_directory() {
    assert_eq!(
        AbsolutePath::current_dir().unwrap().as_path(),
        std::env::current_dir().unwrap()
    );
}

#[test]
fn resolve_canonicalizes_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let typed = AbsolutePath::rooted(&file);
    assert_eq!(
        typed.resolve().unwrap(),
        AbsolutePath::canonicalize(&file).unwrap()
    );
}

#[test]
fn resolve_joins_a_missing_file_onto_its_canonical_parent() {
    let dir = tempfile::tempdir().unwrap();
    let typed = AbsolutePath::rooted(dir.path().join("new.txt"));
    assert_eq!(
        typed.resolve().unwrap().as_path(),
        AbsolutePath::canonicalize(dir.path())
            .unwrap()
            .join("new.txt")
    );
}

#[test]
fn resolve_joins_missing_directories_onto_the_canonical_existing_ancestor() {
    let dir = tempfile::tempdir().unwrap();
    let typed = AbsolutePath::rooted(dir.path().join("no-dir").join("new.txt"));
    let canonical = AbsolutePath::canonicalize(dir.path()).unwrap();
    assert_eq!(
        typed.resolve().unwrap().as_path(),
        canonical.join("no-dir").join("new.txt")
    );
}

#[cfg(unix)]
mod unix;

#[cfg(windows)]
#[test]
fn canonicalize_drops_the_verbatim_drive_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = AbsolutePath::canonicalize(dir.path()).unwrap();
    assert!(
        !canonical.to_string_lossy().starts_with(r"\\?\"),
        "{canonical:?}"
    );
    assert!(canonical.is_absolute());
}
