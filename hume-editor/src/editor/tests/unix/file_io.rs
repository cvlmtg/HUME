use super::super::file_io::{dirty_focused, open_file_buffer};
use super::*;
use pretty_assertions::assert_eq;

#[test]
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
        ed.doc().path(),
        Some(canonical.as_path()),
        ":e same-path must stay on the buffer"
    );
    assert!(
        ed.doc().is_dirty(),
        "dirty flag must be preserved — buffer was not re-read"
    );
}

#[test]
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
        ed.doc().path(),
        Some(canonical.as_path()),
        ":e <deleted-path> must switch to the open buffer"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        "must warn that the file is gone, got: {:?}",
        ed.state.status_msg.as_deref()
    );
}

#[test]
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

/// `:e <missing-path>` reports the path exactly as the user typed it, not its
/// tilde-expanded form — matches `:split`/`:vsplit`
/// (`split_missing_file_error_shows_raw_typed_path` in `multi_pane.rs`), which
/// both share `Editor::resolve_open_path`.
///
/// Uses a `~`-prefixed path rather than a plain relative one: `expand()` is a
/// no-op on inputs with no `~`/env-var sigil, so a plain relative path can't
/// distinguish "show what was typed" from "show the expanded-but-unresolved
/// path". Only an input `expand()` actually rewrites, like `~/...`, proves
/// which one the error message is built from.
#[test]
fn edit_missing_file_error_shows_raw_typed_path() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("e", Some("~/no-such-file-xyz.txt"))
        .unwrap_err();
    assert!(
        err.to_string().starts_with("~/no-such-file-xyz.txt: "),
        "error must lead with the raw typed path, got: {err}"
    );
    assert!(
        !err.to_string()
            .contains(&home.to_string_lossy().to_string()),
        "error must not leak the expanded $HOME path, got: {err}"
    );
}

#[test]
fn edit_relative_path_matches_existing_buffer() {
    // Open a file by absolute path, then :e its basename from the same dir.
    // The lexical-absolute fallback in find_buffer_by_path_arg must match.
    let cwd = CwdSandbox::new();
    let canonical_dir = cwd.path();
    let path = canonical_dir.join("relpath_test.txt");
    std::fs::write(&path, "hello\n").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();

    // Switch to scratch, then :cd to the file's directory.
    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert!(ed.doc().path().is_none(), "must be on scratch");
    ed.execute_typed("cd", Some(canonical_dir.to_str().unwrap()))
        .unwrap();

    // :e with just the basename must switch to the already-open buffer.
    ed.execute_typed("e", Some("relpath_test.txt")).unwrap();
    assert_eq!(
        ed.doc().path(),
        Some(path.as_path()),
        ":e <relative> must switch to the already-open buffer"
    );
}

#[test]
fn open_extra_files_opens_all_paths() {
    let f1 = tempfile::NamedTempFile::new().unwrap();
    let f2 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f1.path(), "file one\n").unwrap();
    std::fs::write(f2.path(), "file two\n").unwrap();

    let canonical1 = std::fs::canonicalize(f1.path()).unwrap();
    let canonical2 = std::fs::canonicalize(f2.path()).unwrap();

    let mut ed = Editor::open(Some(canonical1.clone()), std::sync::Arc::new(|| {})).unwrap();
    let first_id = ed.focused_buffer_id();

    ed.open_extra_files(std::slice::from_ref(&canonical2));

    assert_eq!(ed.state.buffers.len(), 2, "both files must be open");
    assert_eq!(
        ed.focused_buffer_id(),
        first_id,
        "focus must stay on the first file"
    );
    assert_eq!(
        ed.doc().path(),
        Some(canonical1.as_path()),
        "current buffer must still be the first file"
    );
    assert!(
        ed.state.buffers.find_by_path(&canonical2).is_some(),
        "second file must be present in the buffer store"
    );
}

#[test]
fn open_extra_files_deduplicates() {
    let f1 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f1.path(), "hello\n").unwrap();
    let canonical = std::fs::canonicalize(f1.path()).unwrap();

    let mut ed = Editor::open(Some(canonical.clone()), std::sync::Arc::new(|| {})).unwrap();
    // Pass the same path twice — must still result in exactly one buffer.
    ed.open_extra_files(&[canonical.clone(), canonical]);

    assert_eq!(
        ed.state.buffers.len(),
        1,
        "duplicate paths must not open new buffers"
    );
}

#[test]
fn wa_saves_all_dirty_buffers() {
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid1 = ed.focused_buffer_id();
    let (_, meta) = hume_platform::io::read_file(&_tmp1).unwrap();
    ed.doc_mut().file_meta = Some(meta);
    dirty_focused(&mut ed);
    assert!(ed.doc().is_dirty());

    let (_tmp2, bid2) = open_file_buffer(&mut ed, "two\n");
    ed.switch_to_buffer_without_jump(bid2);
    dirty_focused(&mut ed);

    ed.switch_to_buffer_without_jump(bid1);
    ed.execute_typed("wa", None).unwrap();

    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(msg.starts_with("Written"), "got: {msg}");
    assert!(!ed.state.buffers.get(bid1).is_dirty());
    assert_eq!(ed.focused_buffer_id(), bid1);
}

#[test]
fn wa_skips_clean_buffers() {
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid1 = ed.focused_buffer_id();
    let (_, meta) = hume_platform::io::read_file(&_tmp1).unwrap();
    ed.doc_mut().file_meta = Some(meta);
    // bid1 stays clean.

    let (tmp2_path, bid2) = open_file_buffer(&mut ed, "two\n");
    ed.switch_to_buffer_without_jump(bid2);
    dirty_focused(&mut ed);

    let (tmp3_path, _bid3) = open_file_buffer(&mut ed, "three\n");
    ed.switch_to_buffer_without_jump(_bid3);
    dirty_focused(&mut ed);

    ed.switch_to_buffer_without_jump(bid1);
    ed.execute_typed("wa", None).unwrap();

    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.starts_with("Written 2"),
        "expected 2 files written, got: {msg}"
    );
    assert_eq!(std::fs::read_to_string(&_tmp1).unwrap(), "hello\n");
    assert_eq!(std::fs::read_to_string(&*tmp2_path).unwrap(), "xtwo\n");
    assert_eq!(std::fs::read_to_string(&*tmp3_path).unwrap(), "xthree\n");
}

#[test]
fn wa_skips_pathless_buffers() {
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid1 = ed.focused_buffer_id();
    let (_, meta) = hume_platform::io::read_file(&_tmp1).unwrap();
    ed.doc_mut().file_meta = Some(meta);
    dirty_focused(&mut ed);

    // Only one file buffer — scratch shouldn't add to the count.
    let scratch_bid = {
        let scratch = Buffer::new(Text::from("scratch\n"), SelectionSet::default());
        let bid = ed.open_buffer(scratch);
        ed.switch_to_buffer_without_jump(bid);
        dirty_focused(&mut ed);
        bid
    };
    assert!(ed.state.buffers.get(scratch_bid).is_dirty());
    assert!(ed.state.buffers.get(scratch_bid).path().is_none());

    ed.execute_typed("wa", None).unwrap();

    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(msg.starts_with("Written 1"), "expected 1 file, got: {msg}");
    assert!(ed.state.buffers.get(scratch_bid).is_dirty());
    assert!(!ed.state.buffers.get(bid1).is_dirty());
}

#[test]
fn wa_is_noop_if_nothing_dirty() {
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let (_, meta) = hume_platform::io::read_file(&_tmp1).unwrap();
    ed.doc_mut().file_meta = Some(meta);

    ed.execute_typed("wa", None).unwrap();
    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        !msg.starts_with("Written"),
        "no-op must not report written, got: {msg}"
    );
}

#[test]
fn wa_does_not_change_focus() {
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid1 = ed.focused_buffer_id();
    let (_, meta) = hume_platform::io::read_file(&_tmp1).unwrap();
    ed.doc_mut().file_meta = Some(meta);
    dirty_focused(&mut ed);

    let (_tmp2, bid2) = open_file_buffer(&mut ed, "two\n");
    ed.switch_to_buffer_without_jump(bid2);
    dirty_focused(&mut ed);

    ed.switch_to_buffer_without_jump(bid1);
    let before = ed.focused_buffer_id();
    ed.execute_typed("wa", None).unwrap();
    assert_eq!(ed.focused_buffer_id(), before);
}

#[test]
fn wa_preserves_focus_on_single_buffer() {
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid1 = ed.focused_buffer_id();
    let (_, meta) = hume_platform::io::read_file(&_tmp1).unwrap();
    ed.doc_mut().file_meta = Some(meta);
    dirty_focused(&mut ed);

    let before = ed.focused_buffer_id();
    ed.execute_typed("wa", None).unwrap();
    assert_eq!(ed.focused_buffer_id(), before);
    assert!(!ed.state.buffers.get(bid1).is_dirty());
}

/// `:wa` must skip a read-only dirty buffer (e.g. one dirtied by set-text) and
/// still save the remaining writable dirty buffers — no mid-batch abort.
///
/// Fail oracle: remove `&& !buf.is_read_only()` from the typed_write_all filter —
/// write_buffer_by_id returns Err("Buffer is read-only") and the loop propagates
/// it via `?`, leaving bid2 unsaved.
#[test]
fn wa_skips_read_only_dirty_buffer() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    // bid1 — writable dirty buffer backed by a file.
    let (tmp1_path, bid1) = open_file_buffer(&mut ed, "one\n");
    ed.switch_to_buffer_without_jump(bid1);
    dirty_focused(&mut ed);
    assert!(ed.state.buffers.get(bid1).is_dirty());

    // bid2 — a buffer that's been made read-only while dirty.
    let (tmp2_path, bid2) = open_file_buffer(&mut ed, "two\n");
    ed.switch_to_buffer_without_jump(bid2);
    dirty_focused(&mut ed);
    // Simulate the unusual case: set read_only after editing (e.g. set-text path).
    ed.state.buffers.get_mut(bid2).read_only = true;
    assert!(ed.state.buffers.get(bid2).is_dirty());
    assert!(ed.state.buffers.get(bid2).is_read_only());

    ed.switch_to_buffer_without_jump(bid1);
    ed.execute_typed("wa", None).unwrap();

    // bid1 must be saved; bid2 must remain dirty (was skipped, not aborted).
    assert!(
        !ed.state.buffers.get(bid1).is_dirty(),
        "writable buffer must be saved"
    );
    assert!(
        ed.state.buffers.get(bid2).is_dirty(),
        "read-only buffer must remain dirty"
    );
    // File on disk: bid2 content unchanged.
    assert_eq!(
        std::fs::read_to_string(&*tmp2_path).unwrap(),
        "two\n",
        "read-only buffer file must not be touched"
    );
    drop((tmp1_path, tmp2_path));
}

#[test]
fn open_extra_files_nonexistent_logs_warning() {
    let f1 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f1.path(), "hello\n").unwrap();
    let canonical = std::fs::canonicalize(f1.path()).unwrap();

    let mut ed = Editor::open(Some(canonical.clone()), std::sync::Arc::new(|| {})).unwrap();
    let nonexistent = std::path::PathBuf::from("/tmp/hume_test_nonexistent_xyz_404.txt");

    ed.open_extra_files(std::slice::from_ref(&nonexistent));

    assert_eq!(
        ed.state.buffers.len(),
        1,
        "failed open must not add a buffer"
    );
    // Full path, not just the basename: a message that dropped the directory
    // (or substituted some other representation ending in the same filename)
    // would still satisfy a basename-only `contains` check.
    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == crate::editor::message_log::Severity::Warning
                && e.text
                    .starts_with(&format!("Failed to open {}: ", nonexistent.display()))
        }),
        "a warning must be logged for the missing file, with its full path"
    );
}

/// `open_extra_files`'s warning must echo the `PathBuf` it was given verbatim
/// (`path.display()` in `open_extra_files`, not anything derived from the
/// internal `expand()`/canonicalize sequence `resolve_open_path` runs).
///
/// A tilde-literal input is required to prove this: `expand()` is a no-op on
/// inputs with no `~`/env-var sigil, so a plain absolute path (as in
/// `open_extra_files_nonexistent_logs_warning` above) can't tell "warns with
/// the raw arg" apart from "warns with an internally-expanded/resolved
/// path" — both would render identically for such input.
#[test]
fn open_extra_files_warns_with_untransformed_path() {
    let f1 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f1.path(), "hello\n").unwrap();
    let canonical = std::fs::canonicalize(f1.path()).unwrap();
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");

    let mut ed = Editor::open(Some(canonical), std::sync::Arc::new(|| {})).unwrap();
    // Bypass shell tilde expansion by constructing the PathBuf directly —
    // exercises callers (e.g. Steel scripting) that may pass a literal `~`.
    let tilde_path = std::path::PathBuf::from("~/hume-test-no-such-file-xyz.txt");

    ed.open_extra_files(&[tilde_path]);

    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == crate::editor::message_log::Severity::Warning
                && e.text
                    .starts_with("Failed to open ~/hume-test-no-such-file-xyz.txt: ")
        }),
        "warning must echo the raw tilde path, not its expanded $HOME form, got: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .all(|e| !e.text.contains(&home.to_string_lossy().to_string())),
        "warning must not leak the expanded $HOME path"
    );
}

// ── File metadata preservation ────────────────────────────────────────────────

#[test]
fn write_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    // Set a non-default permission that differs from the tempfile default (0600).
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
    // Re-read metadata so file_meta captures the new permissions.
    let (_, meta) = hume_platform::io::read_file(&tmp).unwrap();
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(
        ed.state
            .status_msg
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
    let (_, meta) = hume_platform::io::read_file(&link_path).unwrap();
    assert_eq!(
        meta.resolved_path().to_path_buf(),
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
        ed.state
            .status_msg
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
#[test]
fn write_file_atomic_returns_false_on_plain_write() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "initial\n").unwrap();
    let mut meta = hume_platform::io::read_file_meta(tmp.path()).unwrap();

    let retried = hume_platform::io::write_file_atomic("updated\n", &mut meta, false).unwrap();
    assert!(!retried, "plain write should not trigger chmod-retry");
    assert_eq!(std::fs::read_to_string(tmp.path()).unwrap(), "updated\n");
}

/// `:w!` on a `0o444` target succeeds and preserves the readonly mode on the
/// new inode. Note: on POSIX, `rename(2)` ignores the target file's permission
/// bits when the directory is writable, so the chmod-retry branch in
/// `write_file_atomic` is *not* exercised here — that branch is reached on
/// Windows (READONLY attribute) and exotic filesystems. This test verifies the
/// observable user behaviour either way.
#[test]
fn colon_w_bang_on_readonly_file_preserves_perms() {
    use std::os::unix::fs::PermissionsExt;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    // Make the target readonly and update the buffer's file_meta to match.
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o444)).unwrap();
    let (_, meta) = hume_platform::io::read_file(&tmp).unwrap();
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w!".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    // On POSIX rename succeeds without triggering the chmod-retry path, so the
    // message has no "(forced)" suffix.
    assert_eq!(ed.state.status_msg.as_deref(), Some("Written 1 lines"));
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
    // Permissions must be preserved at 0o444.
    let mode = std::fs::metadata(&tmp).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o444, "0o444 must be preserved on the new inode");
}
