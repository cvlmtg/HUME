use super::*;
use pretty_assertions::assert_eq;

// ── File metadata preservation ────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn write_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    // Set a non-default permission that differs from the tempfile default (0600).
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
    // Re-read metadata so file_meta captures the new permissions.
    let (_, meta) = crate::os::io::read_file(&tmp).unwrap();
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    let mode = std::fs::metadata(&tmp).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "permissions must be preserved across atomic write");
}

#[cfg(unix)]
#[test]
fn write_follows_symlink() {
    use std::os::unix::fs::symlink;

    // Create the real file and a symlink pointing to it.
    let real = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(real.path(), "hello\n").unwrap();

    let link_dir = tempfile::tempdir().unwrap();
    let link_path = link_dir.path().join("link.txt");
    symlink(real.path(), &link_path).unwrap();

    // Open via the symlink — io::read_file should resolve it.
    let (_, meta) = crate::os::io::read_file(&link_path).unwrap();
    assert_eq!(meta.resolved_path, std::fs::canonicalize(real.path()).unwrap());

    let mut ed = editor_from("-[h]>ello\n");
    ed.doc_mut().path = Some(Arc::new(link_path.clone()));
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    // The symlink must still exist and still be a symlink.
    assert!(link_path.symlink_metadata().unwrap().file_type().is_symlink());
    // Content was written to the real file.
    assert_eq!(std::fs::read_to_string(real.path()).unwrap(), "hello\n");
}

// ── :w! force-write ───────────────────────────────────────────────────────────

/// `write_file_atomic` returns `false` (no retry needed) for a normal writable
/// file — verifies the plain-write path of the new return value.
#[cfg(unix)]
#[test]
fn write_file_atomic_returns_false_on_plain_write() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "initial\n").unwrap();
    let meta = crate::os::io::read_file_meta(tmp.path()).unwrap();

    let retried = crate::os::io::write_file_atomic("updated\n", &meta, false).unwrap();
    assert!(!retried, "plain write should not trigger chmod-retry");
    assert_eq!(std::fs::read_to_string(tmp.path()).unwrap(), "updated\n");
}

/// On macOS/Linux `rename(2)` succeeds against a `0o444` target in a writable
/// directory without needing to chmod the target first — the file-level
/// permission is irrelevant to the rename syscall. `:w!` should succeed with
/// the ordinary "Written N lines" message and preserve the `0o444` permissions
/// on the new inode.
#[cfg(unix)]
#[test]
fn write_force_on_readonly_file_preserves_perms() {
    use std::os::unix::fs::PermissionsExt;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    // Make the target readonly and update the buffer's file_meta to match.
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o444)).unwrap();
    let (_, meta) = crate::os::io::read_file(&tmp).unwrap();
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w!".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    // On POSIX rename succeeds without triggering the chmod-retry path, so the
    // message has no "(forced)" suffix.
    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
    // Permissions must be preserved at 0o444.
    let mode = std::fs::metadata(&tmp).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o444, "0o444 must be preserved on the new inode");
}

/// `:wq!` force-writes and then quits. Even for a writable (scratch-free)
/// file, `should_quit` must be `true` after the command.
#[test]
fn colon_wq_bang_force_writes_and_quits() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    for ch in ":wq!".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
    assert!(ed.should_quit);
}

// ── insert-at-selection-start / insert-at-selection-end ──────────────────────

/// `i` with a forward selection collapses to the start of the selection.
#[test]
fn insert_at_selection_start_forward() {
    let mut ed = editor_from("foo -[bar]> baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.mode, Mode::Insert);
}

/// `i` with a backward selection also collapses to the start (lower index).
#[test]
fn insert_at_selection_start_backward() {
    let mut ed = editor_from("foo <[bar]- baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.mode, Mode::Insert);
}

/// `i` with a collapsed cursor just enters insert at the same position.
#[test]
fn insert_at_selection_start_collapsed() {
    let mut ed = editor_from("foo -[b]>ar baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.mode, Mode::Insert);
}

