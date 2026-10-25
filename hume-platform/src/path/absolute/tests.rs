use std::path::Path;

use super::AbsolutePath;

#[test]
fn from_typed_joins_a_relative_path_onto_cwd_and_collapses_dots() {
    let cwd = AbsolutePath::rooted("/home/u");
    let typed = AbsolutePath::from_typed(Path::new("./a/../b.txt"), &cwd).unwrap();
    assert_eq!(typed, AbsolutePath::rooted("/home/u/b.txt"));
}

#[test]
fn from_typed_keeps_an_absolute_path() {
    let path = AbsolutePath::rooted("/srv/data/f.txt");
    let typed = AbsolutePath::from_typed(&path, &AbsolutePath::rooted("/home/u")).unwrap();
    assert_eq!(typed, path);
}

#[cfg(windows)]
#[test]
fn from_typed_rejects_a_drive_relative_path() {
    let cwd = AbsolutePath::rooted(r"C:\home\u");
    let err = AbsolutePath::from_typed(Path::new("D:b.txt"), &cwd).unwrap_err();
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
fn resolve_existing_canonicalizes_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let typed = AbsolutePath::rooted(&file);
    assert_eq!(
        typed.resolve_existing().as_path(),
        std::fs::canonicalize(&file).unwrap()
    );
}

#[test]
fn resolve_existing_joins_a_missing_file_onto_its_canonical_parent() {
    let dir = tempfile::tempdir().unwrap();
    let typed = AbsolutePath::rooted(dir.path().join("new.txt"));
    assert_eq!(
        typed.resolve_existing().as_path(),
        std::fs::canonicalize(dir.path()).unwrap().join("new.txt")
    );
}

#[test]
fn resolve_existing_keeps_a_path_whose_parent_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-dir").join("new.txt");
    let typed = AbsolutePath::rooted(&missing);
    assert_eq!(typed.resolve_existing(), typed);
}

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
