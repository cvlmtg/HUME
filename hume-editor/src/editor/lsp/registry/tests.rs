use super::*;

fn touch(path: &Path) {
    std::fs::write(path, b"").unwrap();
}

#[test]
fn marker_in_immediate_parent_wins() {
    let tmp = tempfile::tempdir().unwrap();
    touch(&tmp.path().join("Cargo.toml"));
    let file = tmp.path().join("src/main.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);

    let root = resolve_root(
        &file,
        &["Cargo.toml".to_string()],
        &PathBuf::from("/should-not-be-used"),
    );
    assert_eq!(root, tmp.path());
}

#[test]
fn marker_in_grandparent_is_found_by_walking_up() {
    let tmp = tempfile::tempdir().unwrap();
    touch(&tmp.path().join("Cargo.toml"));
    let file = tmp.path().join("src/nested/deep.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);

    let root = resolve_root(&file, &["Cargo.toml".to_string()], &PathBuf::from("/cwd"));
    assert_eq!(root, tmp.path());
}

#[test]
fn no_marker_anywhere_falls_back_to_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("src/main.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);
    let cwd = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&cwd).unwrap();

    let root = resolve_root(&file, &["Cargo.toml".to_string()], &cwd);
    assert_eq!(root, cwd);
}

#[test]
fn directory_marker_like_dot_git_is_matched() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".git")).unwrap();
    let file = tmp.path().join("src/main.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);

    let root = resolve_root(&file, &[".git".to_string()], &PathBuf::from("/cwd"));
    assert_eq!(root, tmp.path());
}
