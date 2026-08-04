use super::*;

/// `:vsplit <path>` opens the given file in the new pane instead of mirroring
/// the focused pane's buffer.
#[test]
fn vsplit_path_opens_that_buffer() {
    use hume_engine::pipeline::{Direction, LayoutTree};

    let (path, _tmp_path) = temp_file("other file\n");

    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;

    ed.execute_typed("vsplit", Some(path.to_str().unwrap()))
        .unwrap();

    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_b, pid_a, "focus moves to the new pane");
    let bid_b = ed.view.panes[pid_b].buffer_id;
    assert_ne!(
        bid_b, bid_a,
        "new pane views the opened file, not the original buffer"
    );

    // macOS temp paths differ from their canonical form (/var vs /private/var);
    // canonicalize before comparing against the stored buffer path.
    let canonical = std::fs::canonicalize(&path).unwrap();
    assert_eq!(
        ed.state.buffers.find_by_path(&canonical),
        Some(bid_b),
        "new pane's buffer resolves to the opened file"
    );

    match &ed.view.layout {
        LayoutTree::Split { direction, .. } => assert_eq!(*direction, Direction::Horizontal),
        other => panic!("expected Split layout, got {other:?}"),
    }
}

/// `:split <missing-path>` opens a new-file buffer bound to the path (same
/// `:e`-on-a-missing-file semantics as `Editor::resolve_open_path`) in the new
/// pane, with the display path exactly as the user typed it, not its
/// tilde-expanded form — a symlinked or relative path resolved to an
/// unrecognizable absolute path would otherwise be more confusing, not less.
///
/// Uses a `~`-prefixed path rather than a plain relative one: `expand()` is a
/// no-op on inputs with no `~`/env-var sigil, so a plain relative path (e.g.
/// `./foo.txt`) round-trips identically through both "show what was typed"
/// and "show the expanded-but-unresolved path" — it can't tell the two
/// implementations apart. Only an input `expand()` actually rewrites, like
/// `~/...`, can prove which one the display path is built from.
#[test]
fn split_missing_file_opens_new_file_with_raw_typed_display_path() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    // `resolve_buffer_path` canonicalizes the *parent* dir when the file
    // itself doesn't exist — canonicalize `home` here too, or this
    // assertion can fail on a platform/CI layout where $HOME is itself a
    // symlink.
    let canonical_home = std::fs::canonicalize(&home).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();

    ed.execute_typed("split", Some("~/no-such-file-xyz.txt"))
        .unwrap();

    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_b, bid_a, "new pane views the new-file buffer");
    let buf = ed.state.buffers.get(bid_b);
    assert!(buf.is_new_file());
    assert_eq!(
        buf.display_path(),
        Some("~/no-such-file-xyz.txt"),
        "display path must be the raw typed (collapsed) form"
    );
    assert_eq!(
        buf.path(),
        Some(canonical_home.join("no-such-file-xyz.txt").as_path()),
        "path must resolve against the expanded $HOME, even though display doesn't show it"
    );
}

/// `:split <dir>` must still error, not silently open a new-file buffer —
/// mirrors `edit_directory_path_still_errors` in `file_io.rs`. Also proves
/// the error echoes the raw typed path (`~`), not its expanded `$HOME` form.
#[test]
fn split_directory_path_still_errors_with_raw_typed_path() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("split", Some("~")).unwrap_err();
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

/// Same guarantee as `split_missing_file_opens_new_file_with_raw_typed_display_path`,
/// for `:vsplit`. Both commands share `open_path_arg`, but each has its own
/// dispatch entry point (`typed_split`/`typed_vsplit`), so both are covered.
#[test]
fn vsplit_missing_file_opens_new_file_with_raw_typed_display_path() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    // See `split_missing_file_opens_new_file_with_raw_typed_display_path`'s
    // comment: `resolve_buffer_path` canonicalizes the parent dir.
    let canonical_home = std::fs::canonicalize(&home).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();

    ed.execute_typed("vsplit", Some("~/no-such-file-xyz.txt"))
        .unwrap();

    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_b, bid_a, "new pane views the new-file buffer");
    let buf = ed.state.buffers.get(bid_b);
    assert!(buf.is_new_file());
    assert_eq!(
        buf.display_path(),
        Some("~/no-such-file-xyz.txt"),
        "display path must be the raw typed (collapsed) form"
    );
    assert_eq!(
        buf.path(),
        Some(canonical_home.join("no-such-file-xyz.txt").as_path()),
        "path must resolve against the expanded $HOME, even though display doesn't show it"
    );
}

/// Same guarantee as `split_directory_path_still_errors_with_raw_typed_path`,
/// for `:vsplit`.
#[test]
fn vsplit_directory_path_still_errors_with_raw_typed_path() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("vsplit", Some("~")).unwrap_err();
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

/// `:vsplit <path>` onto a different buffer starts fresh from the global
/// `EditorSettings::wrap_mode`, ignoring the source pane's (unrelated) mode.
#[test]
fn new_file_split_reads_global_wrap_mode() {
    let (path, _tmp_path) = temp_file("other file\n");

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;
    let bid_a = ed.focused_buffer_id();
    ed.view.panes[pid_a].wrap_mode = hume_engine::pane::WrapMode::None;
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::Soft { width: 40 };

    ed.execute_typed("vsplit", Some(path.to_str().unwrap()))
        .unwrap();
    let pid_b = ed.state.focused_pane_id;
    let bid_b = ed.view.panes[pid_b].buffer_id;
    assert_ne!(bid_b, bid_a, "sanity: new pane views a different buffer");

    assert_eq!(
        ed.view.panes[pid_b].wrap_mode,
        hume_engine::pane::WrapMode::Soft { width: 40 },
        "new-file split reads the global default, not the source pane's mode"
    );
}

/// `:vsplit <path>` opens a different buffer in the new pane, so it must
/// start fresh at the buffer's initial selection rather than inheriting the
/// source pane's (unrelated) cursor position.
#[test]
fn split_path_arg_does_not_inherit_source_panes_view() {
    use hume_editing::selection::Selection;

    let (path, _tmp_path) = temp_file("other file\n");

    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    ed.state.panes.state[pid_a][bid_a].selections = SelectionSet::single(Selection::collapsed(2));

    ed.execute_typed("vsplit", Some(path.to_str().unwrap()))
        .unwrap();
    let pid_b = ed.state.focused_pane_id;
    let bid_b = ed.view.panes[pid_b].buffer_id;
    assert_ne!(bid_b, bid_a, "sanity: new pane views a different buffer");

    assert_eq!(
        ed.state.panes.state[pid_b][bid_b].selections,
        ed.state.buffers.get(bid_b).initial_sels(),
        "new pane starts at the opened file's initial selection, not A's cursor"
    );
}

/// A `:vsplit <path>` onto a different buffer keeps the new pane's jump list
/// empty — the source pane's history is irrelevant to a different file.
#[test]
fn split_different_buffer_keeps_empty_jump_list() {
    let (path, _tmp_path) = temp_file("other file\n");

    let mut ed = jump_editor(10);
    let pid_a = ed.state.focused_pane_id;

    // Seed the source pane's jump list with one entry.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    assert_eq!(ed.state.panes.jumps[pid_a].len(), 1);

    ed.execute_typed("vsplit", Some(path.to_str().unwrap()))
        .unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(
        ed.view.panes[pid_b].buffer_id, ed.view.panes[pid_a].buffer_id,
        "sanity: new pane views a different buffer"
    );
    assert_eq!(
        ed.state.panes.jumps[pid_b].len(),
        0,
        "different-buffer split starts with an empty jump list"
    );
}

/// A search match in the focused pane's buffer must never appear in a
/// different pane viewing an unrelated buffer.
///
/// Uses `Editor::open` (not the bare-pane `for_testing` harness) so both
/// panes get real `SharedHighlighter` providers wired via `build_pane` — the
/// bug only reproduces when a pane actually has highlight-reading providers.
#[test]
fn cross_buffer_search_highlight_does_not_bleed_into_other_pane() {
    let (path, _tmp_path) = temp_file("other file\n");

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let pid_a = ed.state.focused_pane_id;

    // Distinguishing content + an active search on buffer A.
    ed.feed_key(key('i'));
    for ch in "foo bar foo".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    ed = ed.with_search_regex("foo");

    // Open a different buffer in a new pane. `:vsplit <path>` moves focus to
    // the new pane; the search stays on buffer A, which is no longer focused.
    ed.execute_typed("vsplit", Some(path.to_str().unwrap()))
        .unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b, "sanity: split created a second pane");
    assert_ne!(
        ed.view.panes[pid_b].buffer_id, ed.view.panes[pid_a].buffer_id,
        "sanity: new pane views a different buffer"
    );

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let a_matches = ed.state.panes.render[pid_a]
        .highlights
        .search
        .read()
        .unwrap()
        .clone();
    assert!(
        !a_matches.is_empty(),
        "sanity: pane A's own search highlights must be populated"
    );

    let b_matches = ed.state.panes.render[pid_b]
        .highlights
        .search
        .read()
        .unwrap()
        .clone();
    assert!(
        b_matches.is_empty(),
        "pane B (different buffer, no search of its own) must not show pane A's matches, got {b_matches:?}"
    );
}
