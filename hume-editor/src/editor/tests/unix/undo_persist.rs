use std::os::unix::fs::PermissionsExt;

use super::super::undo_persist::{Disk, editor_on};
use super::*;
use pretty_assertions::assert_eq;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn saved_history_is_readable_by_its_owner_only() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");

    let undo_file = disk.undo_file();
    assert_eq!(mode(&undo_file), 0o600);
    let undo_dir = disk.state().join("undo");
    for dir in undo_file
        .ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(&undo_dir))
    {
        assert_eq!(mode(dir), 0o700, "{}", dir.display());
    }
}

#[test]
fn a_symlink_and_its_target_share_one_history() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");
    let link = disk.dir.path().join("link.txt");
    std::os::unix::fs::symlink(&disk.file, &link).unwrap();

    let mut through_link = editor_on(&disk, &link, &[]);
    through_link.feed_key(key('u'));
    assert_eq!(text(&through_link), "one\n");
}
