//! Pasting a URL over selected text in a Markdown buffer makes a link.

use super::*;

const URL: &str = "https://bar.baz";

/// `p`'s text after `p` pastes `pasted`.
fn paste(mut p: Preview, pasted: &str) -> String {
    seed_ring(&mut p.ed, &[pasted]);
    p.ed.feed_key(key('p'));
    text(&p.ed)
}

fn pasted_over(marked: &str, pasted: &str) -> String {
    paste(editing(marked), pasted)
}

#[test]
fn p_with_a_url_over_selected_text_makes_a_link() {
    assert_eq!(
        pasted_over("see -[foo]> here\n", URL),
        "see [foo](https://bar.baz) here\n"
    );
}

#[test]
fn a_terminal_paste_of_a_url_over_selected_text_makes_a_link() {
    let mut p = editing("see -[foo]> here\n");
    p.ed.feed_paste(URL);
    assert_eq!(text(&p.ed), "see [foo](https://bar.baz) here\n");
}

#[test]
fn a_classic_paste_of_a_url_over_selected_text_makes_a_link() {
    let mut p = editing("see -[foo]> here\n");
    run(
        &mut p.ed,
        p.tmp.path(),
        "(load-plugin! \"core:classic-paste\")",
    );
    seed_clipboard(&mut p.ed, URL);
    p.ed.feed_key(key_ctrl('v'));
    assert_eq!(text(&p.ed), "see [foo](https://bar.baz) here\n");
}

#[test]
fn a_link_paste_is_one_undo_step() {
    let mut p = editing("see -[foo]> here\n");
    seed_ring(&mut p.ed, &[URL]);
    p.ed.feed_key(key('p'));
    assert_eq!(text(&p.ed), "see [foo](https://bar.baz) here\n");
    p.ed.feed_key(key('u'));
    assert_eq!(state(&p.ed), "see -[foo]> here\n");
}

#[test]
fn text_that_is_not_a_url_replaces_the_selection() {
    assert_eq!(
        pasted_over("see -[foo]> here\n", "bar baz"),
        "see bar baz here\n"
    );
}

#[test]
fn a_url_at_a_cursor_pastes_as_is() {
    assert_eq!(
        pasted_over("see -[f]>oo\n", URL),
        "see fhttps://bar.bazoo\n"
    );
}

#[test]
fn a_url_over_a_url_replaces_it() {
    assert_eq!(
        pasted_over("-[http://old.example]>\n", URL),
        "https://bar.baz\n"
    );
}

#[test]
fn a_url_over_a_link_replaces_it() {
    assert_eq!(pasted_over("-[[foo](old)]>\n", URL), "https://bar.baz\n");
}

#[test]
fn a_url_over_text_spanning_lines_replaces_it() {
    assert_eq!(pasted_over("-[foo\nbar]>\n", URL), "https://bar.baz\n");
}

#[test]
fn a_url_over_text_with_a_bracket_replaces_it() {
    assert_eq!(pasted_over("-[foo]]>\n", URL), "https://bar.baz\n");
    assert_eq!(pasted_over("-[[foo]>\n", URL), "https://bar.baz\n");
}

#[test]
fn a_url_over_text_ending_in_a_backslash_replaces_it() {
    assert_eq!(pasted_over("-[foo\\]>\n", URL), "https://bar.baz\n");
}

#[test]
fn a_url_in_another_language_pastes_as_is() {
    assert_eq!(paste(editing_plain("-[foo]>\n"), URL), "https://bar.baz\n");
}

/// The plugin's editor on `notes/a.md` holding `marked`'s text, selected as
/// `marked` marks it. `notes/` also holds `todo.md`, `my notes.md`,
/// `(draft).md` and the folder `sub/`.
fn editing_in_notes(marked: &str) -> Preview {
    let mut p = preview();
    p.write_notes(&["todo.md", "my notes.md", "(draft).md"]);
    p.open_marked("notes/a.md", marked);
    p
}

fn pasted_in_notes(marked: &str, pasted: &str) -> String {
    paste(editing_in_notes(marked), pasted)
}

#[test]
fn a_path_to_a_file_beside_the_buffer_makes_a_link() {
    assert_eq!(
        pasted_in_notes("see -[foo]> here\n", "todo.md"),
        "see [foo](todo.md) here\n"
    );
}

#[test]
fn a_path_to_a_folder_makes_a_link() {
    assert_eq!(
        pasted_in_notes("see -[foo]> here\n", "sub/"),
        "see [foo](sub/) here\n"
    );
}

#[test]
fn a_path_reaching_above_the_buffers_folder_makes_a_link() {
    assert_eq!(
        pasted_in_notes("see -[foo]> here\n", "../notes/todo.md"),
        "see [foo](../notes/todo.md) here\n"
    );
}

#[test]
fn an_absolute_path_makes_a_link_as_pasted() {
    let p = editing_in_notes("see -[foo]> here\n");
    let path = p.tmp.path().join("notes/todo.md");
    let path = path.to_str().unwrap();
    assert_eq!(paste(p, path), format!("see [foo]({path}) here\n"));
}

#[test]
fn a_path_with_a_space_or_a_parenthesis_makes_an_angle_bracketed_link() {
    assert_eq!(
        pasted_in_notes("see -[foo]> here\n", "my notes.md"),
        "see [foo](<my notes.md>) here\n"
    );
    assert_eq!(
        pasted_in_notes("see -[foo]> here\n", "(draft).md"),
        "see [foo](<(draft).md>) here\n"
    );
}

#[test]
fn a_path_naming_nothing_replaces_the_selection() {
    assert_eq!(
        pasted_in_notes("see -[foo]> here\n", "and/or"),
        "see and/or here\n"
    );
}

#[test]
fn a_path_over_a_link_replaces_it() {
    assert_eq!(pasted_in_notes("-[[foo](old)]>\n", "todo.md"), "todo.md\n");
}

#[test]
fn a_path_over_a_file_name_makes_a_link() {
    assert_eq!(
        pasted_in_notes("see -[todo.md]> here\n", "../notes/todo.md"),
        "see [todo.md](../notes/todo.md) here\n"
    );
}

#[test]
fn a_path_in_an_unsaved_buffer_replaces_the_selection() {
    let mut p = editing_in_notes("-[x]>\n");
    p.open_unsaved_marked("see -[foo]> here\n");
    let path = p.tmp.path().join("notes/todo.md");
    let path = path.to_str().unwrap();
    assert_eq!(paste(p, path), format!("see {path} here\n"));
}
