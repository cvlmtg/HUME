use super::super::command_mode::submit;
use super::*;

/// `:b#` (no space) must switch to the alternate buffer via the minibuf path.
/// The alternate must be reachable even when it has no file name — the
/// `[buffers]` view opened by `:ls` is the canonical pathless case, so the
/// arg must not be expanded to the alternate's path before `:b` sees it
/// (that would error with "Alternate buffer has no file name"). Also covers
/// the `:buffer#` full-alias form.
#[test]
fn colon_b_hash_switches_to_alternate() {
    let f1 = safe_named_tempfile();
    std::fs::write(f1.path(), "file1\n").unwrap();
    let c1 = std::fs::canonicalize(f1.path()).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // focused=scratch, alternate=None
    ed.execute_typed("e", Some(c1.to_str().unwrap())).unwrap();
    // focused=f1, alternate=scratch
    assert_eq!(ed.doc().path(), Some(c1.as_path()));

    // :ls opens the pathless [buffers] view buffer; alternate is now f1.
    submit(&mut ed, "ls");
    assert_eq!(ed.doc().display_name(), "[buffers]");
    assert_eq!(
        ed.alternate_buffer()
            .and_then(|id| ed.state.buffers.get(id).path()),
        Some(c1.as_path()),
    );

    // :b# returns to f1 (alternate has a path — this already worked).
    submit(&mut ed, "b#");
    assert_eq!(ed.doc().path(), Some(c1.as_path()));
    // The alternate is now the pathless [buffers] view — the bug case.
    assert_eq!(
        ed.alternate_buffer()
            .map(|id| ed.state.buffers.get(id).display_name().to_string()),
        Some("[buffers]".to_string()),
    );

    // :b# again must switch to the pathless alternate, not error with
    // "Alternate buffer has no file name".
    submit(&mut ed, "b#");
    assert_eq!(
        ed.doc().display_name(),
        "[buffers]",
        ":b# must switch to the pathless alternate buffer",
    );

    // Ping-pong back to f1 via the full `:buffer#` alias — same path, must work.
    submit(&mut ed, "buffer#");
    assert_eq!(
        ed.doc().path(),
        Some(c1.as_path()),
        ":buffer# must switch to the alternate buffer too",
    );
}

/// `:e! /path` (force + path, no space between `!` and arg) must parse as
/// force=true with the path as argument — regression guard for the new parser.
#[test]
fn colon_edit_bang_path_parses() {
    let f = safe_named_tempfile();
    std::fs::write(f.path(), "clean\n").unwrap();
    let canonical = std::fs::canonicalize(f.path()).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // Open the file first so it's in the buffer list.
    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    // :e! with no space before path must still open/switch correctly.
    let cmd = format!("e!{}", canonical.display());
    submit(&mut ed, &cmd);
    assert_eq!(
        ed.doc().path(),
        Some(canonical.as_path()),
        ":e!<path> (no space) must parse as cmd=e force=true arg=<path>"
    );
}
