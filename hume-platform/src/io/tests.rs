use super::*;
use std::time::Duration;

// FileSignature must compare on size as well as mtime — some filesystems
// (HFS+, FAT) only report mtime to one-second resolution, so a same-second
// rewrite would be invisible to mtime alone. A mtime-only implementation
// would pass every other test in this module but silently miss this one.
#[test]
fn signature_differs_on_size_alone_when_mtime_matches() {
    let mtime = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000));
    let a = FileSignature { mtime, size: 10 };
    let b = FileSignature { mtime, size: 11 };
    assert_ne!(a, b);
}

#[test]
fn signature_equal_when_mtime_and_size_match() {
    let mtime = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000));
    let a = FileSignature { mtime, size: 10 };
    let b = FileSignature { mtime, size: 10 };
    assert_eq!(a, b);
}

#[test]
fn signature_differs_on_earlier_mtime_too() {
    // Equality, not ordering: restoring a file from a backup moves mtime
    // *backwards*, and that must still register as a change.
    let earlier = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(500));
    let later = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000));
    let a = FileSignature {
        mtime: later,
        size: 10,
    };
    let b = FileSignature {
        mtime: earlier,
        size: 10,
    };
    assert_ne!(a, b);
}

#[test]
fn read_signature_reflects_write_file_new() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.txt");

    let meta = write_file_new("hello\n", &path).unwrap();
    let on_disk = read_signature(&path).unwrap();
    assert_eq!(meta.signature(), on_disk);
}

#[test]
fn write_file_atomic_refreshes_signature_so_self_write_is_not_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.txt");
    std::fs::write(&path, "initial\n").unwrap();

    let mut meta = read_file_meta(&path).unwrap();
    write_file_atomic("updated\n", &mut meta, false).unwrap();

    // The rename swaps in a fresh inode, which always changes mtime. If
    // `write_file_atomic` didn't refresh `meta.signature` after persisting,
    // this would spuriously report a change on the very next check.
    let on_disk = read_signature(&path).unwrap();
    assert_eq!(meta.signature(), on_disk);
}

#[test]
#[cfg(unix)]
fn write_file_new_through_dangling_symlink_creates_target_leaves_link() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    symlink(&target, &link).unwrap(); // target does not exist yet — dangling

    let meta = write_file_new("hello\n", &link).unwrap();

    assert!(
        !target.is_symlink() && target.exists(),
        "target must be a real file"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello\n");
    assert_eq!(
        meta.resolved_path(),
        std::fs::canonicalize(&target).unwrap(),
        "FileMeta must key on the resolved target, not the link"
    );
    assert!(
        std::fs::symlink_metadata(&link).unwrap().is_symlink(),
        "the symlink itself must survive the write"
    );
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        target,
        "the symlink must still point at the same target"
    );
}

#[test]
#[cfg(unix)]
fn write_file_new_through_symlink_chain_follows_to_final_target() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("final.txt");
    let middle = dir.path().join("middle.txt");
    let link = dir.path().join("link.txt");
    symlink(&target, &middle).unwrap();
    symlink(&middle, &link).unwrap();

    write_file_new("chain\n", &link).unwrap();

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "chain\n");
    assert!(std::fs::symlink_metadata(&middle).unwrap().is_symlink());
    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
}

#[test]
#[cfg(unix)]
fn write_file_new_symlink_cycle_errors_instead_of_looping() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    symlink(&b, &a).unwrap();
    symlink(&a, &b).unwrap();

    let Err(err) = write_file_new("x\n", &a) else {
        panic!("a symlink cycle must error, not succeed");
    };
    assert!(
        err.to_string().contains("levels of symbolic links"),
        "got: {err}"
    );
}

#[test]
fn write_file_new_on_plain_missing_path_is_unaffected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.txt");

    let meta = write_file_new("hi\n", &path).unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi\n");
    assert_eq!(meta.resolved_path(), std::fs::canonicalize(&path).unwrap());
}
