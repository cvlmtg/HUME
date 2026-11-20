//! Pasting a URL over selected text in a Markdown buffer makes a link.

use super::*;

const URL: &str = "https://bar.baz";

fn pasted_over(marked: &str, pasted: &str) -> String {
    let mut p = editing(marked);
    seed_ring(&mut p.ed, &[pasted]);
    p.ed.feed_key(key('p'));
    text(&p.ed)
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
    let mut p = editing_plain("-[foo]>\n");
    seed_ring(&mut p.ed, &[URL]);
    p.ed.feed_key(key('p'));
    assert_eq!(text(&p.ed), "https://bar.baz\n");
}
