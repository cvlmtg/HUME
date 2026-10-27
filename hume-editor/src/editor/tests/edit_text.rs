//! `(edit-text! pane edits)`: char-range edits from Steel, applied as one
//! edit with every selection carried through it.

use super::*;
use crate::editor::host_impl::EditorHostImpl;
use hume_core::PaneHandle;
use hume_scripting::host::{CharTextEdit, EditHost};
use pretty_assertions::assert_eq;

const PROBE: &str = "edit-text-probe";

/// `ed` with a `PROBE` command whose body calls `edit-text!` with `edits`, a
/// Scheme expression evaluated in the command, where `pane` is bound.
fn define_probe(ed: &mut Editor, tmp: &std::path::Path, edits: &str) {
    run(
        ed,
        tmp,
        &format!(r#"(define-command! "{PROBE}" "" (lambda (pane) (edit-text! pane {edits})))"#),
    );
}

fn run_probe(ed: &mut Editor) {
    ed.execute_keymap_command(PROBE.into(), None, false);
}

/// The marked state after running `edits` on `initial`.
fn after(initial: &str, edits: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from(initial);
    define_probe(&mut ed, tmp.path(), edits);
    run_probe(&mut ed);
    assert_eq!(error_log(&ed), Vec::<String>::new());
    state(&ed)
}

/// The error `edits` reports on `initial`, with the text checked unchanged
/// and the message checked to name `edit-text!`.
fn error_for(initial: &str, edits: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from(initial);
    define_probe(&mut ed, tmp.path(), edits);
    run_probe(&mut ed);
    assert_eq!(state(&ed), initial, "a refused edit changes nothing");
    let errors = error_log(&ed);
    assert_eq!(errors.len(), 1, "one error expected, got {errors:?}");
    let msg = errors.into_iter().next().unwrap_or_default();
    assert!(msg.contains("edit-text!"), "got: {msg}");
    msg
}

fn edit(start: usize, end: usize, text: &str) -> String {
    format!("(hash 'start-char {start} 'end-char {end} 'text {text:?})")
}

fn edits(entries: &[String]) -> String {
    format!("(list {})", entries.join(" "))
}

#[test]
fn a_range_is_replaced() {
    assert_eq!(
        after("-[a]>bc\n", &edits(&[edit(1, 2, "XY")])),
        "-[a]>XYc\n"
    );
}

#[test]
fn an_empty_range_inserts() {
    assert_eq!(
        after("-[a]>bc\n", &edits(&[edit(2, 2, "XY")])),
        "-[a]>bXYc\n"
    );
}

#[test]
fn empty_text_deletes() {
    assert_eq!(after("-[a]>bcd\n", &edits(&[edit(1, 3, "")])), "-[a]>d\n");
}

#[test]
fn every_edit_is_positioned_against_the_text_before_the_call() {
    assert_eq!(
        after("-[a]>bcd\n", &edits(&[edit(3, 4, "D"), edit(1, 2, "BB")])),
        "-[a]>BBcD\n"
    );
}

#[test]
fn a_cursor_at_an_insertion_point_moves_past_the_text() {
    assert_eq!(
        after("a-[b]>c\n", &edits(&[edit(1, 1, "XY")])),
        "aXY-[b]>c\n"
    );
}

#[test]
fn a_cursor_on_replaced_text_lands_at_the_start_of_the_new_text() {
    assert_eq!(
        after("a-[b]>cd\n", &edits(&[edit(1, 3, "XY")])),
        "a-[X]>Yd\n"
    );
}

#[test]
fn a_cursor_inside_a_deletion_lands_where_it_was() {
    assert_eq!(after("a-[b]>cd\n", &edits(&[edit(1, 3, "")])), "a-[d]>\n");
}

#[test]
fn every_selection_is_carried() {
    assert_eq!(
        after(
            "-{a}>b -[c]>d\n",
            &edits(&[edit(0, 0, "#"), edit(3, 3, "#")])
        ),
        "#-{a}>b #-[c]>d\n"
    );
}

#[test]
fn several_edits_are_one_undo_step() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcd\n");
    define_probe(
        &mut ed,
        tmp.path(),
        &edits(&[edit(0, 0, "X"), edit(3, 4, "")]),
    );
    run_probe(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "Xabc\n");
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "abcd\n");
}

#[test]
fn another_panes_selections_are_carried() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    define_probe(&mut ed, tmp.path(), &edits(&[edit(0, 0, "XY")]));
    ed.execute_typed("split", None).unwrap();
    run_probe(&mut ed);
    ed.execute_keymap_command("pane-focus-prev".into(), None, false);
    assert_eq!(state(&ed), "XY-[h]>ello\n");
}

#[test]
fn a_range_edge_inside_a_character_widens_to_cover_it() {
    assert_eq!(
        after("-[a]>e\u{301}b\n", &edits(&[edit(2, 3, "X")])),
        "-[a]>Xb\n"
    );
}

#[test]
fn an_insertion_inside_a_character_moves_to_its_start() {
    assert_eq!(
        after("-[a]>e\u{301}b\n", &edits(&[edit(2, 2, "X")])),
        "-[a]>Xe\u{301}b\n"
    );
}

#[test]
fn text_inserted_at_the_end_gets_its_own_line_break() {
    assert_eq!(after("-[a]>b\n", &edits(&[edit(3, 3, "X")])), "-[a]>b\nX\n");
}

#[test]
fn deleting_everything_keeps_the_final_line_break() {
    assert_eq!(after("ab-[c]>\n", &edits(&[edit(0, 4, "")])), "-[\n]>");
}

#[test]
fn an_insertion_at_the_start_of_a_replacement_goes_first_in_either_order() {
    let expected = "-[a]>iXYd\n";
    assert_eq!(
        after("-[a]>bcd\n", &edits(&[edit(1, 3, "XY"), edit(1, 1, "i")])),
        expected
    );
    assert_eq!(
        after("-[a]>bcd\n", &edits(&[edit(1, 1, "i"), edit(1, 3, "XY")])),
        expected
    );
}

#[test]
fn insertions_around_a_deletion_at_one_offset_go_in_list_order() {
    assert_eq!(
        after(
            "-[a]>bcd\n",
            &edits(&[edit(1, 1, "i"), edit(1, 3, ""), edit(1, 1, "j")])
        ),
        "-[a]>ijd\n"
    );
}

#[test]
fn an_empty_list_changes_nothing() {
    assert_eq!(after("a-[b]>c\n", "(list)"), "a-[b]>c\n");
}

#[test]
fn an_offset_past_the_end_raises() {
    let msg = error_for("-[a]>bc\n", &edits(&[edit(0, 9, "")]));
    assert!(msg.contains("past the end"), "got: {msg}");
}

#[test]
fn a_reversed_range_raises() {
    let msg = error_for("-[a]>bc\n", &edits(&[edit(2, 1, "")]));
    assert!(msg.contains("ends before it starts"), "got: {msg}");
}

#[test]
fn overlapping_edits_raise() {
    let msg = error_for("-[a]>bcd\n", &edits(&[edit(0, 2, ""), edit(1, 3, "")]));
    assert!(msg.contains("overlap"), "got: {msg}");
}

#[test]
fn edges_widened_into_each_other_raise_as_overlapping() {
    let msg = error_for(
        "-[a]>e\u{301}b\n",
        &edits(&[edit(1, 2, "X"), edit(2, 3, "Y")]),
    );
    assert!(msg.contains("overlap"), "got: {msg}");
}

#[test]
fn a_read_only_buffer_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    define_probe(&mut ed, tmp.path(), &edits(&[edit(0, 0, "X")]));
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).read_only = true;
    run_probe(&mut ed);
    assert_eq!(state(&ed), "-[a]>bc\n");
    let errors = error_log(&ed);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("edit-text!") && e.contains("read-only")),
        "got: {errors:?}"
    );
}

#[test]
fn an_insert_mode_binding_edits_without_leaving_insert_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    bind_insert_key(
        &mut ed,
        tmp.path(),
        "ctrl-x",
        &format!("(edit-text! pane {})", edits(&[edit(3, 3, "!")])),
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    assert_eq!(ed.doc().text().to_string(), "abc!\n");
    assert_eq!(ed.state.mode(), Mode::Insert);
    assert_eq!(error_log(&ed), Vec::<String>::new());
}

#[test]
fn an_empty_list_changes_nothing_in_a_read_only_buffer_either() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    define_probe(&mut ed, tmp.path(), "(list)");
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).read_only = true;
    run_probe(&mut ed);
    assert_eq!(state(&ed), "-[a]>bc\n");
    assert_eq!(error_log(&ed), Vec::<String>::new());
}

#[test]
fn a_closed_pane_raises() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    ed.execute_typed("split", None).unwrap();
    let closed = ed.view.focused_pane();
    ed.execute_keymap_command("pane-close".into(), None, false);
    assert!(
        ed.view.panes.get(closed).is_none(),
        "sanity: the pane is gone"
    );

    let err = EditorHostImpl::new(&mut ed.state, &mut ed.view)
        .edit_text(
            PaneHandle::with_pane(bid, closed),
            vec![CharTextEdit {
                start: 0,
                end: 0,
                text: "X".to_string(),
            }],
        )
        .unwrap_err();
    assert!(err.contains("pane has been closed"), "got: {err}");
    assert_eq!(state(&ed), "-[a]>bc\n");
}
