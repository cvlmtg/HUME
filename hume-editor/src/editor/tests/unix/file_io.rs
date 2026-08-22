use super::super::file_io::{dirty_focused, open_file_buffer};
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn edit_existing_buffer_switches_without_reread() {
    let dir = safe_tempdir();
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

/// `:e <path>` re-targeting the buffer it's already focused on is a no-op
/// switch (SPEC.md §4, C5): it raises no `OnBufferEnter` and runs no disk
/// check, matching Vim (`:e` doesn't re-fire `BufEnter` for the buffer
/// you're already on). Deleting the file externally and re-`:e`-ing it while
/// still focused therefore stays silent — the deferred warning still
/// arrives on the next *genuine* buffer-enter.
///
/// Fail oracle: if a no-op `:e` ran a disk check unconditionally (as
/// `enter_buffer_with_jump`'s deleted special case used to), the first
/// assertion below would already see the "no longer exists" warning before
/// any real focus change occurred.
#[test]
fn edit_deleted_file_on_already_focused_buffer_is_silent_until_a_real_buffer_enter() {
    let dir = safe_tempdir();
    let path = dir.path().join("deleted.txt");
    std::fs::write(&path, "content\n").unwrap();
    let canonical = std::fs::canonicalize(&path).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    // Delete from disk while buffer stays open.
    std::fs::remove_file(&canonical).unwrap();
    assert!(!canonical.exists(), "precondition: file must be gone");

    ed.state.status_msg = None;
    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    assert_eq!(
        ed.doc().path(),
        Some(canonical.as_path()),
        ":e <deleted-path> must switch to the open buffer"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "a no-op :e on the already-focused buffer must not run a disk check, got: {:?}",
        ed.state.status_msg.as_deref()
    );

    // A genuine buffer-enter — switch away, then back — still surfaces it.
    let scratch = ed.open_buffer(crate::editor::buffer::Buffer::scratch());
    ed.switch_to_buffer_with_jump(scratch);
    ed.settle();
    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("deleted file's buffer stays open");
    ed.switch_to_buffer_with_jump(bid);
    ed.settle();
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        "the deferred warning must arrive on a genuine buffer-enter, got: {:?}",
        ed.state.status_msg.as_deref()
    );
}

#[test]
fn edit_deleted_file_with_no_buffer_reopens_as_new_file() {
    let dir = safe_tempdir();
    let path = dir.path().join("never_opened.txt");
    // Path never existed — no buffer open for it.
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();

    let canonical_dir = std::fs::canonicalize(dir.path()).unwrap();
    assert_eq!(
        ed.doc().path(),
        Some(canonical_dir.join("never_opened.txt").as_path()),
        ":e <missing-path> must open a buffer bound to the path"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "new-file buffer must start empty (just the structural trailing newline)"
    );
    assert!(
        !ed.doc().is_dirty(),
        "an untouched new-file buffer is clean"
    );
    assert!(
        ed.doc().is_new_file(),
        "buffer must be flagged as not-yet-written"
    );
}

/// `:e <missing-path>` opens a buffer whose display path is exactly the path
/// the user typed, `~`-collapsed — not its tilde-expanded `$HOME` form.
/// Matches `:split`/`:vsplit`
/// (`split_missing_file_opens_new_file_with_raw_typed_display_path` in
/// `multi_pane.rs`), which both share `Editor::resolve_open_path`.
///
/// Uses a `~`-prefixed path rather than a plain relative one: `expand()` is a
/// no-op on inputs with no `~`/env-var sigil, so a plain relative path can't
/// distinguish "typed-derived display form" from "expanded-but-unresolved
/// path". Only an input `expand()` actually rewrites, like `~/...`, proves
/// which one the display path is built from.
#[test]
fn edit_missing_file_shows_raw_typed_display_path() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some("~/no-such-file-xyz.txt"))
        .unwrap();
    assert_eq!(
        ed.doc().display_path(),
        Some("~/no-such-file-xyz.txt"),
        "display path must be the raw typed (collapsed) form"
    );
    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.contains("[new file]"),
        "status message must flag the buffer as new, got: {msg:?}"
    );
    assert!(
        !msg.contains(&home.to_string_lossy().to_string()),
        "status message must not leak the expanded $HOME path, got: {msg:?}"
    );
}

#[test]
fn edit_missing_file_then_write_creates_it() {
    let dir = safe_tempdir();
    let path = dir.path().join("created.txt");

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    assert!(!path.exists(), "opening must not touch disk");

    ed.handle_key(key('i'));
    for ch in "hello".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_esc());
    ed.execute_typed("w", None).unwrap();

    assert!(
        !ed.doc().is_new_file(),
        "buffer must be a real file post-write"
    );
    assert!(!ed.doc().is_dirty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");

    // A second write (now a normal existing-file save) must still succeed.
    ed.handle_key(key('a'));
    ed.handle_key(key('!'));
    ed.handle_key(key_esc());
    ed.execute_typed("w", None).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!\n");
}

/// `:e` with no argument on a new-file buffer has nothing on disk to reload
/// from — it must be a no-op (reported, not silent), not the ENOENT error
/// `reload_from_path` would otherwise raise.
#[test]
fn edit_no_arg_on_new_file_buffer_is_noop() {
    let dir = safe_tempdir();
    let path = dir.path().join("untouched.txt");

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    assert!(ed.doc().is_new_file());

    ed.execute_typed("e", None).unwrap();

    assert!(
        ed.doc().is_new_file(),
        "buffer must still be a pending new-file buffer"
    );
    assert_eq!(ed.doc().text().to_string(), "\n");
    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.contains("new file"),
        "must report the no-op, got: {msg:?}"
    );
}

/// Same, but with unsaved edits and `:e!` — the force path must also be a
/// no-op rather than erroring or discarding the in-memory content (there is
/// nothing on disk to discard *to*).
#[test]
fn edit_force_no_arg_on_dirty_new_file_buffer_is_noop() {
    let dir = safe_tempdir();
    let path = dir.path().join("dirty_new.txt");

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    ed.handle_key(key('i'));
    for ch in "hello".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());

    ed.execute_typed("e", None).unwrap(); // no `!` — the dirty gate must never trigger

    assert!(
        ed.doc().is_new_file(),
        "buffer must still be pending (not written)"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "hello\n",
        "unsaved edits must survive the no-op"
    );
    assert!(!path.exists(), "no-op must not touch disk");
}

#[test]
fn edit_missing_file_twice_dedupes() {
    let dir = safe_tempdir();
    let path = dir.path().join("dedup.txt");

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.execute_typed("b", Some("*scratch*")).unwrap();

    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "second :e on the same missing path must switch to the same buffer"
    );
    assert_eq!(ed.state.buffers.len(), 2, "no duplicate buffer opened");
}

#[test]
fn write_missing_parent_dir_errors_and_leaves_buffer_pending() {
    let dir = safe_tempdir();
    let path = dir.path().join("no-such-subdir").join("file.txt");

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    let err = ed.execute_typed("w", None).unwrap_err();
    assert!(
        err.message().contains("No such file") || err.message().contains("os error"),
        "missing parent dir must surface at write time, got: {}",
        err.message()
    );
    assert!(
        ed.doc().is_new_file(),
        "a failed write must not mark the buffer as saved to disk"
    );
    assert!(!path.exists());
}

/// `:w` on a new-file buffer must re-stat before writing: if the path was
/// created externally after `:e` opened it, that content is foreign, and
/// silently clobbering it would be the exact data loss `stale_write_block`
/// exists to prevent on the existing-file path.
#[test]
fn write_new_file_buffer_refuses_when_file_appeared_externally() {
    let dir = safe_tempdir();
    let path = dir.path().join("appeared.txt");

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // Simulate an external process creating the file after :e.
    std::fs::write(&path, "external content\n").unwrap();

    let err = ed.execute_typed("w", None).unwrap_err();
    assert!(
        err.message().contains("changed on disk"),
        "got: {}",
        err.message()
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "external content\n",
        "the refused write must not touch the externally-created content"
    );
    assert!(
        ed.doc().is_new_file(),
        "a refused write must not mark the buffer as saved"
    );

    // :w! must override and preserve the external file's baseline correctly.
    ed.execute_typed("w!", None).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x\n");
    assert!(!ed.doc().is_new_file());
}

/// A new-file buffer opened via `resolve_buffer_path`'s lexical fallback
/// (parent didn't exist at `:e` time) must have its `path` re-keyed to the
/// fully resolved `FileMeta::resolved_path` once `:w` actually creates the
/// file — otherwise `find_by_path` dedup breaks the moment the file exists.
#[test]
fn write_new_file_buffer_rekeys_path_to_resolved_target() {
    let dir = safe_tempdir();
    let path = dir.path().join("rekeyed.txt");

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.execute_typed("w", None).unwrap();

    let meta = ed
        .doc()
        .file_meta
        .as_ref()
        .expect("file_meta must be set after a successful create");
    assert_eq!(
        ed.doc().path(),
        Some(meta.resolved_path()),
        "buf.path() and file_meta.resolved_path() must agree post-write"
    );
    assert!(
        ed.state
            .buffers
            .find_by_path(meta.resolved_path())
            .is_some(),
        "buffer must be findable by its resolved path"
    );
}

/// A directory target must still error, not silently open a new-file
/// buffer — the one case in `:e`'s open path that's still a genuine
/// failure (`Buffer::from_file_or_new` only tolerates `NotFound`). Also
/// proves the error echoes the raw typed path (`~`), not its expanded
/// `$HOME` form — matches `:e`'s missing-path message
/// (`edit_missing_file_shows_raw_typed_display_path`) — using `~` alone
/// rather than a plain absolute path, since `expand()` is a no-op on inputs
/// with no `~`/env-var sigil and a plain path can't distinguish the two.
#[test]
fn edit_directory_path_still_errors() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("e", Some("~")).unwrap_err();
    assert!(
        err.message().contains("Is a directory") || err.message().contains("os error"),
        "opening a directory must still error, not silently open a new-file buffer, got: {}",
        err.message()
    );
    assert!(
        err.message().starts_with("~: "),
        "error must lead with the raw typed path, got: {}",
        err.message()
    );
    assert!(
        !err.message().contains(&home.to_string_lossy().to_string()),
        "error must not leak the expanded $HOME path, got: {}",
        err.message()
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

/// A new-file buffer opened while an intermediate directory was missing is
/// keyed by `resolve_buffer_path`'s fully-lexical fallback (parent couldn't
/// be canonicalized either) — once that directory appears, re-resolving the
/// same typed string canonicalizes further and would miss a plain
/// `find_by_path` lookup. `:b` (via `find_buffer_by_path_arg`'s lexical
/// fallback) and `:e`'s dedup must still find it, not treat it as unopened.
#[test]
fn buffer_by_path_finds_lexically_keyed_buffer_after_parent_dir_appears() {
    let dir = safe_tempdir();
    let sub = dir.path().join("sub");
    let path = sub.join("f.txt"); // `sub` does not exist yet

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    assert!(ed.doc().is_new_file());
    assert_eq!(ed.state.buffers.len(), 2);

    std::fs::create_dir(&sub).unwrap();

    ed.execute_typed("b", Some("*scratch*")).unwrap();
    ed.execute_typed("b", Some(path.to_str().unwrap())).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        ":b must find the lexically-keyed buffer now that `sub` exists"
    );

    ed.execute_typed("b", Some("*scratch*")).unwrap();
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        ":e must dedupe onto the same buffer, not open a duplicate"
    );
    assert_eq!(ed.state.buffers.len(), 2, "no duplicate buffer opened");
}

#[test]
fn open_extra_files_opens_all_paths() {
    let f1 = safe_named_tempfile();
    let f2 = safe_named_tempfile();
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

/// `hume newfile.txt` (the *first* CLI file argument, `Editor::open`'s own
/// `Buffer::from_file(path)?` branch — distinct from `open_extra_files`,
/// which only handles trailing args) must open a new-file buffer instead of
/// exiting on ENOENT.
#[test]
fn startup_with_missing_first_file_opens_new_file_buffer() {
    let dir = safe_tempdir();
    let path = dir.path().join("startup_new.txt");

    let ed = Editor::open(Some(path.clone()), std::sync::Arc::new(|| {})).unwrap();

    assert_eq!(ed.state.buffers.len(), 1);
    assert!(ed.doc().is_new_file());
    let canonical_dir = std::fs::canonicalize(dir.path()).unwrap();
    assert_eq!(
        ed.doc().path(),
        Some(canonical_dir.join("startup_new.txt").as_path())
    );
}

#[test]
fn open_extra_files_deduplicates() {
    let f1 = safe_named_tempfile();
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

/// A new-file buffer (`:e` on a missing path, not yet written) must be
/// written by `:wa` like any other dirty buffer, not silently skipped for
/// lacking a `file_meta` baseline.
#[test]
fn wa_saves_new_file_buffers() {
    let dir = safe_tempdir();
    let new_path = dir.path().join("wa_new.txt");

    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid1 = ed.focused_buffer_id();
    let (_, meta) = hume_platform::io::read_file(&_tmp1).unwrap();
    ed.doc_mut().file_meta = Some(meta);
    dirty_focused(&mut ed);

    ed.execute_typed("e", Some(new_path.to_str().unwrap()))
        .unwrap();
    let bid2 = ed.focused_buffer_id();
    assert!(ed.doc().is_new_file());
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());

    ed.execute_typed("wa", None).unwrap();

    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(msg.starts_with("Written 2"), "got: {msg}");
    assert!(!ed.state.buffers.get(bid1).is_dirty());
    assert!(!ed.state.buffers.get(bid2).is_dirty());
    assert!(!ed.state.buffers.get(bid2).is_new_file());
    assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "x\n");
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
fn open_extra_files_nonexistent_opens_new_file_buffer() {
    let f1 = safe_named_tempfile();
    std::fs::write(f1.path(), "hello\n").unwrap();
    let canonical = std::fs::canonicalize(f1.path()).unwrap();

    let mut ed = Editor::open(Some(canonical.clone()), std::sync::Arc::new(|| {})).unwrap();
    let dir = safe_tempdir();
    let nonexistent = dir.path().join("hume_test_nonexistent_xyz_404.txt");
    let canonical_dir = std::fs::canonicalize(dir.path()).unwrap();

    ed.open_extra_files(std::slice::from_ref(&nonexistent));

    assert_eq!(
        ed.state.buffers.len(),
        2,
        "a trailing missing CLI path must open a new-file buffer, not warn"
    );
    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical_dir.join("hume_test_nonexistent_xyz_404.txt"))
        .expect("new-file buffer must be findable by its resolved path");
    assert!(ed.state.buffers.get(bid).is_new_file());
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("Failed to open")),
        "must not warn for a missing path — it opens instead"
    );
    // Info severity reports to `status_msg`, not `message_log` (see
    // `Severity::Info`'s routing).
    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.contains("hume_test_nonexistent_xyz_404.txt") && msg.contains("[new file]"),
        "must report the new-file buffer, matching :e's own message, got: {msg:?}"
    );
}

/// The new-file buffer's display path must be the `PathBuf` `open_extra_files`
/// was given, not anything derived from the internal `expand()`/canonicalize
/// sequence `resolve_open_path` runs.
///
/// A tilde-literal input is required to prove this: `expand()` is a no-op on
/// inputs with no `~`/env-var sigil, so a plain absolute path (as in
/// `open_extra_files_nonexistent_opens_new_file_buffer` above) can't tell
/// "typed-derived display form" apart from "expanded-but-unresolved path" —
/// both would render identically for such input.
#[test]
fn open_extra_files_new_file_shows_untransformed_display_path() {
    let f1 = safe_named_tempfile();
    std::fs::write(f1.path(), "hello\n").unwrap();
    let canonical = std::fs::canonicalize(f1.path()).unwrap();
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    // `resolve_buffer_path` canonicalizes the *parent* dir when the file
    // itself doesn't exist — canonicalize `home` here too, or this lookup
    // can miss on a platform/CI layout where $HOME is itself a symlink.
    let canonical_home = std::fs::canonicalize(&home).unwrap();

    let mut ed = Editor::open(Some(canonical), std::sync::Arc::new(|| {})).unwrap();
    // Bypass shell tilde expansion by constructing the PathBuf directly —
    // exercises callers (e.g. Steel scripting) that may pass a literal `~`.
    let tilde_path = std::path::PathBuf::from("~/hume-test-no-such-file-xyz.txt");

    ed.open_extra_files(&[tilde_path]);

    assert_eq!(ed.state.buffers.len(), 2);
    let display = ed
        .state
        .buffers
        .find_by_path(&canonical_home.join("hume-test-no-such-file-xyz.txt"))
        .map(|bid| ed.state.buffers.get(bid).display_path().unwrap().to_owned());
    assert_eq!(
        display.as_deref(),
        Some("~/hume-test-no-such-file-xyz.txt"),
        "display path must be the raw typed (collapsed) form, not the expanded $HOME one"
    );
}

/// `open_extra_files`'s warning must echo the `PathBuf` it was given verbatim
/// (`path.display()` in `open_extra_files`, not anything derived from the
/// internal `expand()`/canonicalize sequence `resolve_open_path` runs).
///
/// A tilde-literal input is required to prove this: `expand()` is a no-op on
/// inputs with no `~`/env-var sigil, so a plain absolute path can't tell
/// "warns with the raw arg" apart from "warns with an internally-expanded/
/// resolved path" — both would render identically for such input. Targets
/// `~` (home dir) itself, not a missing filename under it: a missing path
/// now opens a new-file buffer instead of warning (see
/// `open_extra_files_nonexistent_opens_new_file_buffer`) — a directory is
/// still a genuine open failure, so it's the only case left that still
/// warns.
#[test]
fn open_extra_files_warns_with_untransformed_path() {
    let f1 = safe_named_tempfile();
    std::fs::write(f1.path(), "hello\n").unwrap();
    let canonical = std::fs::canonicalize(f1.path()).unwrap();
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");

    let mut ed = Editor::open(Some(canonical), std::sync::Arc::new(|| {})).unwrap();
    let tilde_path = std::path::PathBuf::from("~");

    ed.open_extra_files(&[tilde_path]);

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning && e.text.starts_with("Failed to open ~: ")),
        "warning must echo the raw tilde path, not its expanded $HOME form, got: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains(&home.to_string_lossy().to_string())),
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
    let real = safe_named_tempfile();
    std::fs::write(real.path(), "hello\n").unwrap();

    let link_dir = safe_tempdir();
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
    let tmp = safe_named_tempfile();
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
