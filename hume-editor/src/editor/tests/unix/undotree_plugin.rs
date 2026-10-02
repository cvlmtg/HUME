// core:undotree: renderer and end-to-end plugin tests.
//
// `render.scm` is evaluated through a probe plugin staged next to a copy of
// the real shipped file: the probe's one command shows the rendered rows of
// a literal node list in the drawer, and the tests read them back. Every
// expected row set below is derived by hand from the lane rules in the
// plugin README, never by calling the renderer.

use super::*;
use hume_scripting::ScriptingHost;

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
    let guard = HumeRuntimeGuard::new();
    let plugin_dir = guard.runtime.path().join("plugins/core/undotree-probe");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("render.scm"), RENDER_SCM).unwrap();
    let probe = format!(
        r#"(require "render.scm")
(define-command! "undotree-probe" "Show the rendered rows of a literal node list."
  (lambda (pane)
    (show-drawer-list! pane (hash-ref (undotree/render {}) 'rows) (lambda (i) (begin)))))"#,
        scheme_nodes(nodes)
    );
    std::fs::write(plugin_dir.join("plugin.scm"), probe).unwrap();

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        "(load-plugin! \"core:undotree-probe\")",
        tmp.path(),
    );
    ed.scripting = Some(host);
    ed.execute_keymap_command("undotree-probe".to_string().into(), None, false);
    ed.settle();
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

// ── End to end: the real core:undotree plugin ───────────────────────────────

use std::path::Path;
use std::time::Duration;

fn setup(tmp: &Path) -> (Editor, RealRuntimeGuard) {
    let guard = RealRuntimeGuard::new();
    let mut ed = editor_from("-[h]>ello\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &hume_scripting::eager_load_scm("core:undotree", None),
        tmp,
    );
    ed.scripting = Some(host);
    ed.settle();
    (ed, guard)
}

fn toggle(ed: &mut Editor) {
    ed.execute_keymap_command("toggle-undotree".to_string().into(), None, false);
    ed.settle();
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

    insta::assert_snapshot!(masked_rows(&ed), @r"
    o    @  ##
    | o     ##
    o-'   S ##
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
    drain_until(&mut ed, |ed| {
        drawer_rows(ed).get(1).is_some_and(|row| row.contains('@'))
    });

    assert_eq!(ed.doc().text().to_string(), "ello\n");
    insta::assert_snapshot!(masked_rows(&ed), @r"
    o       ##
    | o  @  ##
    o-'   S ##
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
    drain_until(&mut ed, |ed| drawer_rows(ed).len() == 4);

    insta::assert_snapshot!(masked_rows(&ed), @r"
    o    @  ##
    o       ##
    | o     ##
    o-'   S ##
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
    drain_until(&mut ed, |ed| {
        drawer_rows(ed).last().is_some_and(|row| row.contains('@'))
    });

    assert_eq!(ed.doc().text().to_string(), before, "setup: no text change");
    insta::assert_snapshot!(masked_rows(&ed), @r"
    o     ##
    o     ##
    o  @S ##
    ");
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
    drain_until(&mut ed, |ed| drawer_rows(ed).len() == 1);

    insta::assert_snapshot!(masked_rows(&ed), @"o  @S ##");
}

#[test]
fn undotree_typed_command_opens_the_drawer() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path());

    type_cmd(&mut ed, ":undotree");
    ed.settle();

    assert!(drawer_open(&ed));
    insta::assert_snapshot!(masked_rows(&ed), @"o  @S ##");
}

const PLUGIN_SCM: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/undotree/plugin.scm"
));
const MANIFEST_SCM: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/undotree/manifest.scm"
));

#[test]
fn undotree_ages_refresh_while_the_drawer_is_open() {
    let guard = HumeRuntimeGuard::new();
    let plugin_dir = guard.runtime.path().join("plugins/core/undotree");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("render.scm"), RENDER_SCM).unwrap();
    std::fs::write(plugin_dir.join("manifest.scm"), MANIFEST_SCM).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        PLUGIN_SCM.replace(
            "(define undotree/age-refresh-ms 60000)",
            "(define undotree/age-refresh-ms 100)",
        ),
    )
    .unwrap();
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &hume_scripting::eager_load_scm("core:undotree", None),
        tmp.path(),
    );
    ed.scripting = Some(host);
    ed.settle();
    toggle(&mut ed);
    assert!(
        drawer_rows(&ed).iter().all(|row| row.ends_with(" 0s")),
        "setup: a fresh tree reads 0s"
    );

    drain_until(&mut ed, |ed| {
        drawer_rows(ed).iter().all(|row| row.ends_with(" 1s"))
    });
}
