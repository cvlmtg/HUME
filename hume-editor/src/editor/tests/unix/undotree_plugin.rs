// core:undotree: renderer and end-to-end plugin tests.
//
// `render.scm` is evaluated through a probe plugin staged next to a copy of
// the real shipped file: the probe's one command shows the rendered rows of
// a literal node list in the drawer, and the tests read them back. Every
// expected row set below is derived by hand from the lane rules in the
// plugin README, never by calling the renderer.

use super::*;

const RENDER_SCM: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/undotree/render.scm"
));

struct Node {
    id: usize,
    parent: Option<usize>,
    age_secs: u64,
    current: bool,
    saved: bool,
}

fn node(id: usize, parent: Option<usize>, age_secs: u64, current: bool, saved: bool) -> Node {
    Node {
        id,
        parent,
        age_secs,
        current,
        saved,
    }
}

/// A Scheme list of the `(buffer-undo-tree pane)` hashes for `nodes`.
fn scheme_nodes(nodes: &[Node]) -> String {
    let hashes: Vec<String> = nodes
        .iter()
        .map(|n| {
            let parent = n.parent.map_or("#f".to_string(), |p| p.to_string());
            format!(
                "(hash 'id {} 'parent {parent} 'age-secs {} 'current? {} 'saved? {})",
                n.id,
                n.age_secs,
                if n.current { "#t" } else { "#f" },
                if n.saved { "#t" } else { "#f" },
            )
        })
        .collect();
    format!("(list {})", hashes.join(" "))
}

/// The drawer rows `undotree/render` produces for `nodes`, joined by newlines.
fn rendered_rows(nodes: &[Node]) -> String {
    rendered_rows_after(&[], nodes)
}

/// [`rendered_rows`], with `earlier` rendered first, in order, in the same
/// Steel VM.
fn rendered_rows_after(earlier: &[&[Node]], nodes: &[Node]) -> String {
    let guard = RuntimeDirs::new();
    let plugin_dir = guard.runtime.path().join("plugins/core/undotree-probe");
    let earlier: Vec<String> = earlier
        .iter()
        .map(|nodes| format!("(undotree/render {})", scheme_nodes(nodes)))
        .collect();
    let probe = format!(
        r#"(require "render.scm")
(define-command! "undotree-probe" "Show the rendered rows of a literal node list."
  (lambda (pane)
    {}
    (let ([rendered (undotree/render {})])
      (show-drawer-list! pane (hash-ref rendered 'nodes) (lambda (i tok) (begin))
                         #:render (hash-ref rendered 'render)))))"#,
        earlier.join("\n    "),
        scheme_nodes(nodes)
    );
    write_core_plugin(&guard, "undotree-probe", &probe);
    std::fs::write(plugin_dir.join("render.scm"), RENDER_SCM).unwrap();

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = guard.dirs();
    run(
        &mut ed,
        tmp.path(),
        "(load-plugin! \"core:undotree-probe\")",
    );
    ed.execute_keymap_command("undotree-probe".to_string().into(), None, false);
    frame(&mut ed, 80, 40);
    drawer_rows(&ed).join("\n")
}

#[test]
fn render_linear_chain() {
    let rows = rendered_rows(&[
        node(0, None, 600, false, true),
        node(1, Some(0), 300, false, false),
        node(2, Some(1), 90, true, false),
    ]);
    insta::assert_snapshot!(rows);
}

#[test]
fn render_branch_merges_into_fork_point() {
    let rows = rendered_rows(&[
        node(0, None, 720, false, true),
        node(1, Some(0), 540, false, false),
        node(2, Some(1), 420, false, false),
        node(3, Some(2), 360, false, false),
        node(4, Some(1), 120, true, false),
        node(5, Some(0), 240, false, false),
    ]);
    insta::assert_snapshot!(rows);
}

#[test]
fn render_crossing_lane_passes_through_merge_bus() {
    let rows = rendered_rows(&[
        node(0, None, 300, false, true),
        node(1, Some(0), 240, false, false),
        node(2, Some(1), 180, false, false),
        node(3, Some(0), 120, false, false),
        node(4, Some(1), 60, true, false),
    ]);
    insta::assert_snapshot!(rows);
}

#[test]
fn render_age_units() {
    let ages = [172_800, 86_400, 86_399, 3_600, 3_599, 60, 59, 0];
    let nodes: Vec<Node> = ages
        .iter()
        .enumerate()
        .map(|(id, &age)| node(id, id.checked_sub(1), age, id == ages.len() - 1, id == 0))
        .collect();
    insta::assert_snapshot!(rendered_rows(&nodes));
}

/// The markers and ages come from the second call's nodes even though its
/// tree has the first call's shape.
#[test]
fn render_same_shape_shows_the_later_markers_and_ages() {
    let rows = rendered_rows_after(
        &[&[
            node(0, None, 600, false, true),
            node(1, Some(0), 300, false, false),
            node(2, Some(1), 90, true, false),
        ]],
        &[
            node(0, None, 7200, true, false),
            node(1, Some(0), 30, false, false),
            node(2, Some(1), 5, false, true),
        ],
    );
    insta::assert_snapshot!(rows, @r"
    o   S  5s
    o     30s
    o  @   2h
    ");
}

/// Same revision count, different parents: the second call draws its own
/// branch, not the first call's chain.
#[test]
fn render_new_parents_redraw_the_graph() {
    let rows = rendered_rows_after(
        &[&[
            node(0, None, 120, false, true),
            node(1, Some(0), 60, false, false),
            node(2, Some(1), 0, true, false),
        ]],
        &[
            node(0, None, 120, false, true),
            node(1, Some(0), 60, false, false),
            node(2, Some(0), 0, true, false),
        ],
    );
    insta::assert_snapshot!(rows, @"
    o    @   0s
    | o      1m
    o-'   S  2m
    ");
}

/// Seconds until `undotree/render`'s soonest age label change for `ages`, as
/// the probe command traces it.
fn next_change_secs(ages: &[u64]) -> String {
    let guard = RuntimeDirs::new();
    let plugin_dir = guard.runtime.path().join("plugins/core/undotree-probe");
    let nodes: Vec<Node> = ages
        .iter()
        .enumerate()
        .map(|(id, &age)| node(id, id.checked_sub(1), age, id == 0, false))
        .collect();
    let probe = format!(
        r#"(require "render.scm")
(define-command! "undotree-probe" "Trace the seconds to the next age label change."
  (lambda (pane)
    (log! 'trace (number->string (hash-ref (undotree/render {}) 'next-change-secs)))))"#,
        scheme_nodes(&nodes)
    );
    write_core_plugin(&guard, "undotree-probe", &probe);
    std::fs::write(plugin_dir.join("render.scm"), RENDER_SCM).unwrap();

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = guard.dirs();
    run(
        &mut ed,
        tmp.path(),
        "(load-plugin! \"core:undotree-probe\")",
    );
    ed.execute_keymap_command("undotree-probe".to_string().into(), None, false);
    ed.settle();
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == crate::editor::message_log::Severity::Trace)
        .last()
        .map(|e| e.text.to_string())
        .expect("the probe traced a value")
}

/// The soonest label change across rows: seconds-unit rows change every
/// second, a minutes-unit row when its next whole minute is reached, and so
/// on for hours and days.
#[test]
fn render_reports_the_seconds_to_the_next_age_label_change() {
    assert_eq!(next_change_secs(&[7200, 125, 5]), "1");
    assert_eq!(next_change_secs(&[7300, 125]), "55");
    assert_eq!(next_change_secs(&[7300, 3599]), "1");
    assert_eq!(next_change_secs(&[172_800, 90_000]), "82800");
}

/// A fork off the root, the newest revision (3) on one branch and the saved
/// root on the other.
fn forked() -> Vec<Node> {
    vec![
        node(0, None, 40, false, true),
        node(1, Some(0), 30, false, false),
        node(2, Some(1), 20, false, false),
        node(3, Some(0), 10, true, false),
    ]
}

/// `forked()` with revision 4 added under the newest, as one more edit does.
fn forked_with_child() -> Vec<Node> {
    vec![
        node(0, None, 40, false, true),
        node(1, Some(0), 30, false, false),
        node(2, Some(1), 20, false, false),
        node(3, Some(0), 10, false, false),
        node(4, Some(3), 0, true, false),
    ]
}

/// Rows derived by hand from the lane rules: 4 and 3 stack in column 0, 2 and
/// 1 sit in column 1 beside the lane still waiting for 0, which merges at 0.
#[test]
fn render_a_child_of_the_newest_revision_extends_the_graph() {
    let rows = rendered_rows_after(&[&forked()], &forked_with_child());
    insta::assert_snapshot!(rows, @"
    o    @   0s
    o       10s
    | o     20s
    | o     30s
    o-'   S 40s
    ");
}

/// Each of two edits in a row extends the previous layout.
#[test]
fn render_two_appended_edits_match_a_fresh_layout() {
    let mut longer = forked_with_child();
    longer[4].current = false;
    longer.push(node(5, Some(4), 0, true, false));
    assert_eq!(
        rendered_rows_after(&[&forked(), &forked_with_child()], &longer),
        rendered_rows(&longer)
    );
}

/// A new revision under an older one, such as an edit after an undo, draws a
/// new branch rather than extending the newest revision's column.
#[test]
fn render_a_child_of_an_older_revision_matches_a_fresh_layout() {
    let mut branched = forked();
    branched[3].current = false;
    branched.push(node(4, Some(2), 0, true, false));
    assert_eq!(
        rendered_rows_after(&[&forked()], &branched),
        rendered_rows(&branched)
    );
}

// ── End to end: the real core:undotree plugin ───────────────────────────────

use std::path::Path;
use std::time::Duration;

fn setup(tmp: &Path) -> (Editor, RealRuntimeDirs) {
    let guard = RealRuntimeDirs::new();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = guard.dirs();
    run(
        &mut ed,
        tmp,
        &hume_scripting::eager_load_scm("core:undotree", None),
    );
    ed.settle();
    (ed, guard)
}

fn toggle(ed: &mut Editor) {
    ed.execute_keymap_command("toggle-undotree".to_string().into(), None, false);
    render(ed);
}

/// Two branches off the root: rev1 deletes `h`, rev2 deletes `e`; the editor
/// ends on rev2, with the root saved.
fn two_branches(ed: &mut Editor) {
    for ch in ['d', 'u', 'l', 'd'] {
        ed.feed_key(key(ch));
    }
    ed.settle();
}

fn is_age(token: &str) -> bool {
    let (digits, unit) = token.split_at(token.len().saturating_sub(1));
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && matches!(unit, "s" | "m" | "h" | "d")
}

/// The drawer rows with each age replaced by `#`s of the same width: ages
/// are wall-clock, everything else in a row is a function of the tree.
fn masked_rows(ed: &Editor) -> String {
    drawer_rows(ed)
        .iter()
        .map(|row| {
            row.split(' ')
                .map(|token| {
                    if is_age(token) {
                        "#".repeat(token.len())
                    } else {
                        token.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn drawer_open(ed: &Editor) -> bool {
    ed.state.input.drawer().is_some()
}

#[test]
fn undotree_opens_on_current_revision() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    two_branches(&mut ed);

    toggle(&mut ed);

    insta::assert_snapshot!(masked_rows(&ed), @"
    o    @   ##
    | o      ##
    o-'   S  ##
    ");
    assert_eq!(ed.state.input.drawer().unwrap().selected, 0);
}

#[test]
fn undotree_enter_jumps_and_rerenders_current_marker() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    two_branches(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "hllo\n");
    toggle(&mut ed);

    ed.handle_key(key_shift_down());
    ed.handle_key(key_enter());
    drain_frames_until(&mut ed, |ed| {
        drawer_rows(ed).get(1).is_some_and(|row| row.contains('@'))
    });

    assert_eq!(ed.doc().text().to_string(), "ello\n");
    insta::assert_snapshot!(masked_rows(&ed), @"
    o        ##
    | o  @   ##
    o-'   S  ##
    ");
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        1,
        "the highlight stays on the revision that was jumped to"
    );
}

#[test]
fn undotree_redo_after_enter_follows_jumped_branch() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    two_branches(&mut ed);
    toggle(&mut ed);
    ed.handle_key(key_shift_down());
    ed.handle_key(key_enter());
    ed.settle();

    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    ed.feed_key(key('U'));

    assert_eq!(
        ed.doc().text().to_string(),
        "ello\n",
        "redo continues along the jumped-to branch, not the newest one"
    );
}

#[test]
fn undotree_refreshes_after_edit() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    two_branches(&mut ed);
    toggle(&mut ed);

    ed.feed_key(key('d'));
    drain_frames_until(&mut ed, |ed| drawer_rows(ed).len() == 4);

    insta::assert_snapshot!(masked_rows(&ed), @"
    o    @   ##
    o        ##
    | o      ##
    o-'   S  ##
    ");
}

#[test]
fn undotree_saved_marker_follows_a_write() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    let path = tmp.path().join("saved.txt");
    std::fs::write(&path, "hello\n").unwrap();
    ed.feed_key(key('d'));
    ed.settle();
    toggle(&mut ed);
    insta::assert_snapshot!(masked_rows(&ed), @"
    o  @   ##
    o   S  ##
    ");

    ed.execute_typed("w", Some(path.to_str().unwrap())).unwrap();
    drain_frames_until(&mut ed, |ed| {
        drawer_rows(ed).first().is_some_and(|row| row.contains('S'))
    });

    insta::assert_snapshot!(masked_rows(&ed), @"
    o  @S  ##
    o      ##
    ");
}

#[test]
fn undotree_refreshes_after_net_identity_undo() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    for key_event in [key('i'), key('x'), key_esc(), key('d')] {
        ed.feed_key(key_event);
    }
    ed.settle();
    toggle(&mut ed);
    let before = ed.doc().text().to_string();

    ed.feed_key(key('2'));
    ed.feed_key(key('u'));
    drain_frames_until(&mut ed, |ed| {
        drawer_rows(ed).last().is_some_and(|row| row.contains('@'))
    });

    assert_eq!(ed.doc().text().to_string(), before, "setup: no text change");
    insta::assert_snapshot!(masked_rows(&ed), @"
    o      ##
    o      ##
    o  @S  ##
    ");
}

// ── Revision diff ────────────────────────────────────────────────────────────

const DIFF_SOURCE: &str = "undotree";

/// `setup`, with `core:git-diff` (and the `core:stdlib` it needs) loaded
/// before `core:undotree`.
fn setup_with_git_diff(tmp: &Path) -> (Editor, RealRuntimeDirs) {
    let guard = RealRuntimeDirs::new();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = guard.dirs();
    let load = format!(
        "(load-plugin! \"core:stdlib\")\n{}\n{}",
        hume_scripting::eager_load_scm("core:git-diff", None),
        hume_scripting::eager_load_scm("core:undotree", None),
    );
    run(&mut ed, tmp, &load);
    ed.settle();
    (ed, guard)
}

/// `(line, before, text, segment char ranges)`.
type DiffVline = (usize, bool, String, Vec<(usize, usize)>);

/// The undotree source's virtual lines on `bid`.
fn diff_vlines(ed: &Editor, bid: BufferId) -> Vec<DiffVline> {
    let text = ed.state.buffers.get(bid).text();
    ed.state
        .config
        .decorations
        .virtual_lines_for(DIFF_SOURCE, bid)
        .iter()
        .map(|e| {
            (
                text.char_to_line(e.pos).index(),
                e.before,
                e.text.clone(),
                e.segments
                    .iter()
                    .map(|(start, end, _)| (start.index(), end.index()))
                    .collect(),
            )
        })
        .collect()
}

fn diff_line_bgs(ed: &Editor, bid: BufferId) -> usize {
    ed.state
        .config
        .decorations
        .line_backgrounds_for(DIFF_SOURCE, bid)
        .len()
}

/// After `two_branches` the editor is on rev2 ("hllo"), whose parent is the
/// root ("hello"): the diff shows the `e` rev2 removed, on the root's line.
#[test]
fn undotree_draws_the_current_revisions_diff_on_open() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_git_diff(tmp.path());
    two_branches(&mut ed);
    let bid = ed.focused_buffer_id();

    toggle(&mut ed);

    assert_eq!(
        diff_vlines(&ed, bid),
        vec![(0, true, "hello".to_string(), vec![(1, 2)])]
    );
    assert_eq!(diff_line_bgs(&ed, bid), 1);
}

#[test]
fn undotree_enter_redraws_the_diff_for_the_revision_jumped_to() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_git_diff(tmp.path());
    two_branches(&mut ed);
    let bid = ed.focused_buffer_id();
    toggle(&mut ed);

    ed.handle_key(key_shift_down());
    ed.handle_key(key_enter());
    drain_frames_until(&mut ed, |ed| {
        diff_vlines(ed, bid) == vec![(0, true, "hello".to_string(), vec![(0, 1)])]
    });

    assert_eq!(ed.doc().text().to_string(), "ello\n");
}

/// The jump's text change and its diff land in one settle, so no frame shows
/// the new text under the previous revision's diff.
#[test]
fn undotree_enter_draws_the_jumped_to_revisions_diff_in_the_same_settle() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_git_diff(tmp.path());
    two_branches(&mut ed);
    let bid = ed.focused_buffer_id();
    toggle(&mut ed);

    ed.handle_key(key_shift_down());
    ed.handle_key(key_enter());
    ed.settle();

    assert_eq!(ed.doc().text().to_string(), "ello\n");
    assert_eq!(
        diff_vlines(&ed, bid),
        vec![(0, true, "hello".to_string(), vec![(0, 1)])]
    );
}

/// From the root, `U` redoes into rev2 ("hllo"): its diff and the `@` on its
/// row appear in the same settle as the redo.
#[test]
fn undotree_redo_in_buffer_redraws_diff_and_marker_in_the_same_settle() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_git_diff(tmp.path());
    two_branches(&mut ed);
    ed.feed_key(key('u'));
    ed.settle();
    let bid = ed.focused_buffer_id();
    toggle(&mut ed);
    assert!(
        diff_vlines(&ed, bid).is_empty(),
        "setup: the root has no diff"
    );

    ed.feed_key(key('U'));
    render(&mut ed);

    assert_eq!(ed.doc().text().to_string(), "hllo\n");
    assert_eq!(
        diff_vlines(&ed, bid),
        vec![(0, true, "hello".to_string(), vec![(1, 2)])]
    );
    assert!(drawer_rows(&ed)[0].contains('@'));
}

#[test]
fn undotree_root_has_no_diff() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_git_diff(tmp.path());
    two_branches(&mut ed);
    let bid = ed.focused_buffer_id();
    toggle(&mut ed);
    assert!(!diff_vlines(&ed, bid).is_empty(), "setup: a diff is drawn");

    ed.handle_key(key_shift_down());
    ed.handle_key(key_shift_down());
    ed.handle_key(key_enter());
    drain_frames_until(&mut ed, |ed| diff_vlines(ed, bid).is_empty());

    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert_eq!(diff_line_bgs(&ed, bid), 0);
}

#[test]
fn undotree_toggle_close_clears_the_diff() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_git_diff(tmp.path());
    two_branches(&mut ed);
    let bid = ed.focused_buffer_id();
    toggle(&mut ed);
    assert!(!diff_vlines(&ed, bid).is_empty(), "setup: a diff is drawn");

    toggle(&mut ed);

    assert!(diff_vlines(&ed, bid).is_empty());
    assert_eq!(diff_line_bgs(&ed, bid), 0);
}

#[test]
fn undotree_esc_clears_the_diff() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_git_diff(tmp.path());
    two_branches(&mut ed);
    let bid = ed.focused_buffer_id();
    toggle(&mut ed);
    assert!(!diff_vlines(&ed, bid).is_empty(), "setup: a diff is drawn");

    ed.handle_key(key_esc());
    ed.settle();

    assert!(diff_vlines(&ed, bid).is_empty());
    assert_eq!(diff_line_bgs(&ed, bid), 0);
}

/// Copies every `.scm` file of the shipped core plugin `name` into the
/// guard's runtime.
fn copy_core_plugin_files(guard: &RuntimeDirs, name: &str) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../runtime/plugins/core")
        .join(name);
    let to = guard.runtime.path().join("plugins/core").join(name);
    std::fs::create_dir_all(&to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "scm") {
            std::fs::copy(&path, to.join(path.file_name().unwrap())).unwrap();
        }
    }
}

/// A renderer that raises while clearing still ends the session, so the next
/// toggle opens the drawer again.
#[test]
fn undotree_session_ends_when_clearing_the_diff_raises() {
    let guard = RuntimeDirs::new();
    copy_core_plugin_files(&guard, "stdlib");
    copy_core_plugin_files(&guard, "git-diff");
    copy_core_plugin_files(&guard, "undotree");
    let plugin_scm = guard
        .runtime
        .path()
        .join("plugins/core/git-diff/plugin.scm");
    let patched = std::fs::read_to_string(&plugin_scm).unwrap().replace(
        "  git-diff/release-source!)",
        "  (lambda (pane source) (error \"clear failed\")))",
    );
    std::fs::write(&plugin_scm, patched).unwrap();

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = guard.dirs();
    let load = format!(
        "(load-plugin! \"core:stdlib\")\n{}\n{}",
        hume_scripting::eager_load_scm("core:git-diff", None),
        hume_scripting::eager_load_scm("core:undotree", None),
    );
    run(&mut ed, tmp.path(), &load);
    ed.settle();
    two_branches(&mut ed);
    toggle(&mut ed);
    assert!(drawer_open(&ed), "setup: the drawer is open");

    toggle(&mut ed);
    assert!(!drawer_open(&ed), "the failed toggle closed the drawer");

    toggle(&mut ed);
    assert!(drawer_open(&ed), "the next toggle opens it again");
}

/// A refresh that finds the buffer on the same revision leaves the diff
/// alone: the age timer re-renders the rows each second, and the diff the
/// test cleared behind the plugin's back stays cleared until a jump moves the
/// revision.
#[test]
fn undotree_redraws_the_diff_only_when_the_revision_changes() {
    let tmp = safe_tempdir();
    let _guard = RealRuntimeDirs::new();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = _guard.dirs();
    let load = format!(
        "(load-plugin! \"core:stdlib\")\n{}\n{}\n{}",
        hume_scripting::eager_load_scm("core:git-diff", None),
        hume_scripting::eager_load_scm("core:undotree", None),
        r#"(define-command! "clear-diff-probe" "Clear the undotree diff."
             (lambda (pane) (set-virtual-lines! "undotree" pane (list))))"#,
    );
    run(&mut ed, tmp.path(), &load);
    ed.settle();
    two_branches(&mut ed);
    let bid = ed.focused_buffer_id();
    toggle(&mut ed);
    assert!(!diff_vlines(&ed, bid).is_empty(), "setup: a diff is drawn");

    ed.execute_keymap_command("clear-diff-probe".to_string().into(), None, false);
    ed.settle();
    assert!(
        diff_vlines(&ed, bid).is_empty(),
        "setup: the diff is cleared"
    );

    let before = drawer_rows(&ed);
    drain_frames_until(&mut ed, |ed| drawer_rows(ed) != before);
    assert!(
        diff_vlines(&ed, bid).is_empty(),
        "the timer refreshed the rows without a revision change, so no redraw"
    );

    ed.handle_key(key_shift_down());
    ed.handle_key(key_enter());
    drain_frames_until(&mut ed, |ed| !diff_vlines(ed, bid).is_empty());
}

/// Showing another buffer retargets the drawer to it, and the buffer it left
/// stops showing a diff.
#[test]
fn undotree_buffer_switch_clears_the_diff_of_the_buffer_left() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_git_diff(tmp.path());
    two_branches(&mut ed);
    let first = ed.focused_buffer_id();
    toggle(&mut ed);
    assert!(
        !diff_vlines(&ed, first).is_empty(),
        "setup: a diff is drawn"
    );

    let other = tmp.path().join("other.txt");
    std::fs::write(&other, "x\n").unwrap();
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();
    drain_frames_until(&mut ed, |ed| diff_vlines(ed, first).is_empty());

    assert_ne!(
        ed.focused_buffer_id(),
        first,
        "setup: the other buffer is shown"
    );
    assert_eq!(diff_line_bgs(&ed, first), 0);
}

/// Without `core:git-diff` the drawer works as before, draws no diff and says
/// once, in the status line, how to get one.
#[test]
fn undotree_without_git_diff_opens_with_a_notice_and_no_diff() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    two_branches(&mut ed);
    let bid = ed.focused_buffer_id();

    toggle(&mut ed);

    assert!(drawer_open(&ed));
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("core:git-diff")),
        "status: {:?}",
        ed.state.status_msg
    );
    assert!(diff_vlines(&ed, bid).is_empty());
    let errors: Vec<_> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == crate::editor::message_log::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "no errors expected, got {errors:?}");

    toggle(&mut ed);
    assert!(!drawer_open(&ed), "closing still works without a renderer");
}

#[test]
fn undotree_toggle_closes() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());

    toggle(&mut ed);
    assert!(drawer_open(&ed));
    toggle(&mut ed);
    assert!(!drawer_open(&ed));
    toggle(&mut ed);
    assert!(drawer_open(&ed), "a third toggle opens a fresh drawer");
}

#[test]
fn undotree_esc_ends_session() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    toggle(&mut ed);

    ed.handle_key(key_esc());
    ed.settle();
    assert!(!drawer_open(&ed));

    ed.feed_key(key('d'));
    std::thread::sleep(Duration::from_millis(300));
    ed.settle();
    assert!(
        !drawer_open(&ed),
        "an edit after Esc must not reopen the drawer"
    );

    toggle(&mut ed);
    assert!(drawer_open(&ed));
    assert_eq!(drawer_rows(&ed).len(), 2, "the reopened drawer is current");
}

#[test]
fn undotree_follows_buffer_switch() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    two_branches(&mut ed);
    toggle(&mut ed);
    assert_eq!(drawer_rows(&ed).len(), 3);

    ed.open_buffer(Buffer::at_start(BufferText::from("other\n")));
    ed.execute_typed("bn", None).unwrap();
    drain_frames_until(&mut ed, |ed| drawer_rows(ed).len() == 1);

    insta::assert_snapshot!(masked_rows(&ed), @"o  @S  ##");
}

#[test]
fn undotree_typed_command_opens_the_drawer() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());

    type_cmd(&mut ed, ":undotree");
    render(&mut ed);

    assert!(drawer_open(&ed));
    insta::assert_snapshot!(masked_rows(&ed), @"o  @S  ##");
}

#[test]
fn undotree_typed_command_rejects_an_argument() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());

    type_cmd(&mut ed, ":undotree foo");
    render(&mut ed);

    assert!(!drawer_open(&ed), "the rejected command opened nothing");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text == "`:undotree` takes no argument"),
        "the rejection is reported"
    );
}

/// Patches the shipped `core:git-diff` so `git-diff/render-diff` raises.
fn copy_plugins_with_raising_renderer(guard: &RuntimeDirs) {
    copy_core_plugin_files(guard, "stdlib");
    copy_core_plugin_files(guard, "git-diff");
    copy_core_plugin_files(guard, "undotree");
    let plugin_scm = guard
        .runtime
        .path()
        .join("plugins/core/git-diff/plugin.scm");
    let patched = std::fs::read_to_string(&plugin_scm).unwrap().replace(
        "  git-diff/draw-for-source!)",
        "  (lambda (pane source hunks) (error \"draw failed\")))",
    );
    std::fs::write(&plugin_scm, patched).unwrap();
}

/// A renderer that raises on draw still leaves a session: the next toggle
/// closes the drawer instead of opening another over it.
#[test]
fn undotree_session_survives_a_raising_diff_renderer() {
    let guard = RuntimeDirs::new();
    copy_plugins_with_raising_renderer(&guard);
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = guard.dirs();
    let load = format!(
        "(load-plugin! \"core:stdlib\")\n{}\n{}",
        hume_scripting::eager_load_scm("core:git-diff", None),
        hume_scripting::eager_load_scm("core:undotree", None),
    );
    run(&mut ed, tmp.path(), &load);
    ed.settle();
    two_branches(&mut ed);

    toggle(&mut ed);
    assert!(drawer_open(&ed), "setup: the drawer is open");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("draw failed")),
        "the renderer's error is reported"
    );

    toggle(&mut ed);
    assert!(!drawer_open(&ed), "the toggle closed the drawer");
}

/// Two panes show one buffer. The drawer is opened from the first and focus
/// then moves to the second: Enter jumps through the focused pane, so its
/// selections come from the revision and the first pane's are carried along.
#[test]
fn undotree_enter_acts_through_the_focused_pane() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    two_branches(&mut ed);
    let first = ed.state.focus.id();
    toggle(&mut ed);
    ed.execute_typed("split", None).unwrap();
    let second = ed.state.focus.id();
    assert_ne!(first, second, "setup: focus moved to the new pane");

    ed.handle_key(key_shift_down());
    ed.handle_key(key_enter());
    drain_frames_until(&mut ed, |ed| {
        drawer_rows(ed).get(1).is_some_and(|row| row.contains('@'))
    });

    assert_eq!(ed.doc().text().to_string(), "ello\n");
    assert_eq!(
        state(&ed),
        "-[e]>llo\n",
        "the focused pane has the revision's own selections"
    );
}

/// The pane the drawer was opened from has closed and the focused pane shows
/// the same buffer: one Enter jumps, with no retarget step first.
#[test]
fn undotree_enter_jumps_after_the_session_pane_closes() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    two_branches(&mut ed);
    toggle(&mut ed);
    ed.execute_typed("split", None).unwrap();
    ed.execute_keymap_command("pane-focus-next".to_string().into(), None, false);
    ed.execute_keymap_command("pane-close".to_string().into(), None, false);
    ed.settle();
    assert!(drawer_open(&ed), "setup: the drawer survives the close");

    ed.handle_key(key_shift_down());
    ed.handle_key(key_enter());
    drain_frames_until(&mut ed, |ed| {
        drawer_rows(ed).get(1).is_some_and(|row| row.contains('@'))
    });

    assert_eq!(ed.doc().text().to_string(), "ello\n");
}

#[test]
fn undotree_ages_refresh_while_the_drawer_is_open() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());
    toggle(&mut ed);
    let before = drawer_rows(&ed);

    drain_frames_until(&mut ed, |ed| drawer_rows(ed) != before);
}

fn setup_with_foreign_drawer(tmp: &Path) -> (Editor, RealRuntimeDirs) {
    let guard = RealRuntimeDirs::new();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = guard.dirs();
    let source = format!(
        r#"{}
(define-command! "foreign-drawer" "Open an unrelated drawer."
  (lambda (pane)
    (show-drawer-list! pane (list "foreign") (lambda (i tok) (begin)))))"#,
        hume_scripting::eager_load_scm("core:undotree", None)
    );
    run(&mut ed, tmp, &source);
    ed.settle();
    (ed, guard)
}

fn open_foreign_drawer(ed: &mut Editor) {
    ed.execute_keymap_command("foreign-drawer".to_string().into(), None, false);
}

/// Another feature's drawer replaces the tree's; the history change that
/// follows must leave the foreign rows alone.
#[test]
fn undotree_session_ends_when_another_drawer_replaces_it() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_foreign_drawer(tmp.path());
    toggle(&mut ed);
    open_foreign_drawer(&mut ed);
    ed.settle();
    assert_eq!(drawer_rows(&ed), ["foreign"]);

    ed.feed_key(key('d'));
    ed.settle();
    render(&mut ed);

    assert_eq!(drawer_rows(&ed), ["foreign"]);
}

/// The replaced drawer's `#f` is still queued when the toggle opens a new
/// tree: the new session must survive it and keep following history.
#[test]
fn undotree_new_session_survives_the_replaced_drawers_queued_close() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_with_foreign_drawer(tmp.path());
    toggle(&mut ed);
    open_foreign_drawer(&mut ed);
    toggle(&mut ed);
    ed.settle();
    assert_eq!(drawer_rows(&ed).len(), 1, "setup: the new tree is open");

    ed.feed_key(key('d'));
    ed.settle();
    render(&mut ed);

    assert_eq!(drawer_rows(&ed).len(), 2, "the new session saw the edit");
}
