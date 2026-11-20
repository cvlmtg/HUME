//! Completing the path in a Markdown link's destination.

use super::*;

/// The plugin's editor on `notes/<name>` holding `marked`'s text, its
/// open announced, in Insert mode before the cursor `marked` marks. `notes/` also holds
/// `todo.md`, `.hidden` and `sub/b.md`.
fn inserting(name: &str, marked: &str) -> Preview {
    let mut p = preview();
    p.write_notes(&["todo.md", ".hidden", "sub/b.md"]);
    p.open_marked(&format!("notes/{name}"), marked);
    p.ed.feed_key(key('i'));
    p
}

#[test]
fn ctrl_space_in_a_destination_lists_the_files_beside_the_buffer() {
    let mut p = inserting("a.md", "[x](-[\n]>");
    trigger_completion(&mut p.ed);
    assert_eq!(menu_labels(&p.ed), ["a.md", "sub/", "todo.md"]);
}

#[test]
fn typing_narrows_the_paths_to_the_typed_start() {
    let mut p = inserting("a.md", "[x](-[\n]>");
    trigger_completion(&mut p.ed);
    p.ed.feed_key(key('t'));
    p.ed.settle();
    assert_eq!(menu_labels(&p.ed), ["todo.md"]);
}

#[test]
fn typing_matches_a_path_fuzzily() {
    let mut p = inserting("a.md", "[x](-[\n]>");
    trigger_completion(&mut p.ed);
    p.ed.feed_keys([key('d'), key('m')]);
    p.ed.settle();
    assert_eq!(menu_labels(&p.ed), ["todo.md"]);
}

#[test]
fn typing_past_the_first_character_reads_the_folder_once() {
    let mut p = inserting("a.md", "[x](-[\n]>");
    trigger_completion(&mut p.ed);
    p.ed.feed_key(key('t'));
    p.ed.settle();
    std::fs::write(p.tmp.path().join("notes/toast.md"), "").unwrap();
    p.ed.feed_key(key('o'));
    p.ed.settle();
    assert_eq!(menu_labels(&p.ed), ["todo.md"]);
}

#[test]
fn a_directory_in_the_destination_lists_its_files() {
    let mut p = inserting("a.md", "[x](sub/-[\n]>");
    trigger_completion(&mut p.ed);
    assert_eq!(menu_labels(&p.ed), ["b.md"]);
}

#[test]
fn accepting_a_path_inserts_it_after_its_directory() {
    let mut p = inserting("a.md", "[x](sub/-[\n]>");
    trigger_completion(&mut p.ed);
    p.ed.feed_key(key_enter());
    assert_eq!(text(&p.ed), "[x](sub/b.md\n");
}

#[test]
fn accepting_a_directory_inserts_its_name_without_the_slash() {
    let mut p = inserting("a.md", "[x](-[\n]>");
    trigger_completion(&mut p.ed);
    p.ed.feed_key(key('s'));
    p.ed.settle();
    p.ed.feed_key(key_enter());
    assert_eq!(text(&p.ed), "[x](sub\n");
}

#[test]
fn a_parent_directory_lists_its_files() {
    let mut p = inserting("sub/c.md", "[x](../-[\n]>");
    trigger_completion(&mut p.ed);
    assert_eq!(menu_labels(&p.ed), ["sub/", "todo.md"]);
}

#[test]
fn a_dot_lists_the_hidden_files() {
    let mut p = inserting("a.md", "[x](.-[\n]>");
    trigger_completion(&mut p.ed);
    assert_eq!(menu_labels(&p.ed), [".hidden"]);
}

#[test]
fn typing_an_opening_parenthesis_after_link_text_opens_the_menu() {
    let mut p = inserting("a.md", "[x]-[\n]>");
    p.ed.feed_key(key('('));
    p.ed.settle();
    assert_eq!(menu_labels(&p.ed), ["a.md", "sub/", "todo.md"]);
}

#[test]
fn typing_a_slash_after_a_directory_opens_its_files() {
    let mut p = inserting("a.md", "[x](sub-[\n]>");
    p.ed.feed_key(key('/'));
    p.ed.settle();
    assert_eq!(menu_labels(&p.ed), ["b.md"]);
}

#[test]
fn ctrl_space_outside_a_destination_lists_no_paths() {
    let mut p = inserting("a.md", "[x] (-[\n]>");
    trigger_completion(&mut p.ed);
    assert_eq!(menu_labels(&p.ed), Vec::<String>::new());
}

#[test]
fn a_destination_from_the_root_lists_no_paths() {
    let mut p = inserting("a.md", "[x](/-[\n]>");
    trigger_completion(&mut p.ed);
    assert_eq!(menu_labels(&p.ed), Vec::<String>::new());
}

#[test]
fn an_opening_parenthesis_in_plain_text_opens_no_menu() {
    let mut p = inserting("a.md", "call-[\n]>");
    p.ed.feed_key(key('('));
    p.ed.settle();
    assert!(p.ed.state.input.buffer_completion().is_none());
}

#[test]
fn a_buffer_with_no_file_lists_no_paths() {
    let mut p = inserting("a.md", "-[x]>\n");
    p.ed.feed_key(key_esc());
    p.open_unsaved_marked("[x](-[\n]>");
    p.ed.feed_key(key('i'));
    trigger_completion(&mut p.ed);
    assert_eq!(menu_labels(&p.ed), Vec::<String>::new());
}
