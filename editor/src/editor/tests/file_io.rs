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

    for ch in ":w".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(
        ed.status_msg
            .as_deref()
            .unwrap_or("")
            .starts_with("Written")
    );
    let mode = std::fs::metadata(&tmp).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o644,
        "permissions must be preserved across atomic write"
    );
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
    assert_eq!(
        meta.resolved_path,
        std::fs::canonicalize(real.path()).unwrap()
    );

    let mut ed = editor_from("-[h]>ello\n");
    ed.doc_mut().set_path(Some(link_path.clone()));
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(
        ed.status_msg
            .as_deref()
            .unwrap_or("")
            .starts_with("Written")
    );
    // The symlink must still exist and still be a symlink.
    assert!(
        link_path
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // Content was written to the real file.
    assert_eq!(std::fs::read_to_string(real.path()).unwrap(), "hello\n");
}

// ── :w! force-write ───────────────────────────────────────────────────────────

/// `write_file_atomic` returns `false` (no retry needed) for a normal writable
/// file — verifies the plain-write path of the new return value.
#[cfg(unix)]
#[test]
fn write_file_atomic_returns_false_on_plain_write() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "initial\n").unwrap();
    let meta = crate::os::io::read_file_meta(tmp.path()).unwrap();

    let retried = crate::os::io::write_file_atomic("updated\n", &meta, false).unwrap();
    assert!(!retried, "plain write should not trigger chmod-retry");
    assert_eq!(std::fs::read_to_string(tmp.path()).unwrap(), "updated\n");
}

/// `:w!` on a `0o444` target succeeds and preserves the readonly mode on the
/// new inode. Note: on POSIX, `rename(2)` ignores the target file's permission
/// bits when the directory is writable, so the chmod-retry branch in
/// `write_file_atomic` is *not* exercised here — that branch is reached on
/// Windows (READONLY attribute) and exotic filesystems. This test verifies the
/// observable user behaviour either way.
#[cfg(unix)]
#[test]
fn colon_w_bang_on_readonly_file_preserves_perms() {
    use std::os::unix::fs::PermissionsExt;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    // Make the target readonly and update the buffer's file_meta to match.
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o444)).unwrap();
    let (_, meta) = crate::os::io::read_file(&tmp).unwrap();
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w!".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    // On POSIX rename succeeds without triggering the chmod-retry path, so the
    // message has no "(forced)" suffix.
    assert_eq!(ed.status_msg.as_deref(), Some("Written 1 lines"));
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
    for ch in ":wq!".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.status_msg.as_deref(), Some("Written 1 lines"));
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

// ── :e on already-open buffers ────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn edit_existing_buffer_switches_without_reread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.txt");
    std::fs::write(&path, "original\n").unwrap();
    let canonical = std::fs::canonicalize(&path).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    // Dirty the in-memory buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "buffer must be dirty before :e");

    // :e <same-path> on an already-open buffer must switch without re-reading.
    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(canonical.as_path()),
        ":e same-path must stay on the buffer"
    );
    assert!(
        ed.doc().is_dirty(),
        "dirty flag must be preserved — buffer was not re-read"
    );
}

#[test]
#[cfg(not(windows))]
fn edit_deleted_file_with_open_buffer_switches_and_warns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deleted.txt");
    std::fs::write(&path, "content\n").unwrap();
    let canonical = std::fs::canonicalize(&path).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    // Delete from disk while buffer stays open.
    std::fs::remove_file(&canonical).unwrap();
    assert!(!canonical.exists(), "precondition: file must be gone");

    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(canonical.as_path()),
        ":e <deleted-path> must switch to the open buffer"
    );
    assert!(
        ed.status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        "must warn that the file is gone, got: {:?}",
        ed.status_msg.as_deref()
    );
}

#[test]
#[cfg(not(windows))]
fn edit_deleted_file_with_no_buffer_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("never_opened.txt");
    // Path never existed — no buffer open for it.
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("e", Some(path.to_str().unwrap()))
        .unwrap_err();
    assert!(
        err.to_string().contains("No such file") || err.to_string().contains("os error"),
        "must error with ENOENT when no buffer is open, got: {err}"
    );
}

#[test]
#[cfg(not(windows))]
fn edit_relative_path_matches_existing_buffer() {
    // Open a file by absolute path, then :e its basename from the same dir.
    // The lexical-absolute fallback in find_buffer_by_path_arg must match.
    let _cwd = CwdGuard::new();
    let dir = tempfile::tempdir().unwrap();
    let canonical_dir = std::fs::canonicalize(&dir).unwrap();
    let path = canonical_dir.join("relpath_test.txt");
    std::fs::write(&path, "hello\n").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();

    // Switch to scratch, then :cd to the file's directory.
    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert!(ed.doc().path.is_none(), "must be on scratch");
    ed.execute_typed("cd", Some(canonical_dir.to_str().unwrap()))
        .unwrap();

    // :e with just the basename must switch to the already-open buffer.
    ed.execute_typed("e", Some("relpath_test.txt")).unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(path.as_path()),
        ":e <relative> must switch to the already-open buffer"
    );
}
