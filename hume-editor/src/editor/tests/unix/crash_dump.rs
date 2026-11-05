use std::os::unix::fs::PermissionsExt;

use super::super::file_io::dirty_focused;
use super::*;
use pretty_assertions::assert_eq;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The folder scratch dumps go to is owner-only, and the data directory
/// created above it keeps the mode any new directory gets.
#[test]
fn the_scratch_dump_folder_is_owner_only_and_its_parent_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let dumps = data.join("dumps");
    let mut ed = editor_from("-[h]>ello\n");
    dirty_focused(&mut ed);

    let outcomes = ed.dump_dirty_buffers(Some(&dumps));

    assert!(outcomes[0].1.is_ok(), "{outcomes:?}");
    assert_eq!(mode(&dumps), 0o700);
    let sibling = tmp.path().join("sibling");
    std::fs::create_dir(&sibling).unwrap();
    assert_eq!(mode(&data), mode(&sibling));
}
