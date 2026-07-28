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
