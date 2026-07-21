use super::*;
use tempfile::TempDir;

#[test]
fn new_creates_servers_dir() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("hume");
    // data_dir does not exist yet; ScriptDirs::new should create it.
    let dirs = ScriptDirs::new(Some(data_dir.clone()), None);
    assert!(data_dir.join("servers").is_dir());
    assert!(dirs.servers_dir().is_ok());
}

#[test]
fn servers_dir_errs_when_dirs_unavailable() {
    let dirs = ScriptDirs::new(None, None);
    assert!(dirs.servers_dir().is_err());
}

#[test]
fn servers_dir_succeeds_when_data_dir_available() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("hume");
    let dirs = ScriptDirs::new(Some(data_dir.clone()), None);
    let servers = std::fs::canonicalize(data_dir.join("servers")).unwrap();
    assert_eq!(dirs.servers_dir().unwrap(), servers);
}
