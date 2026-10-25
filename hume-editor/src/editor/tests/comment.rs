//! `toggle-comment` through the key path: `G G`, its refusals, and the
//! language each selection resolves to.

use super::*;

use hume_editing::comment::{BlockCommentTokens, LineCommentTokens};
use hume_treesitter::registry::{ConfigureOutcome, ConfiguredBy, LanguageFields};

/// An editor over `src` whose buffer is a language named `name`, with
/// `line` and `block` configured as its comment tokens.
fn editor_in(src: &str, name: &str, line: &[&str], block: &[(&str, &str)]) -> Editor {
    let mut ed = editor_from(src);
    super::language::attach_host(&mut ed, "");
    let languages = &mut ed.state.config.languages;
    let id = languages.register_identity_no_rebuild(name, &[], &[], &[]);
    let fields = LanguageFields {
        comment_tokens: (!line.is_empty()).then(|| LineCommentTokens::from_strs(line)),
        block_comment_tokens: (!block.is_empty()).then(|| BlockCommentTokens::from_pairs(block)),
        ..LanguageFields::default()
    };
    let outcomes = languages.configure_fields(id, fields, ConfiguredBy::User);
    assert!(
        outcomes
            .iter()
            .all(|&(_, outcome)| outcome == ConfigureOutcome::Applied),
        "{outcomes:?}"
    );
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some(id));
    ed
}

fn rust_editor(src: &str) -> Editor {
    editor_in(src, "rust", &["//"], &[("/*", "*/")])
}

fn toggle(ed: &mut Editor) {
    ed.feed_keys([key('G'), key('G')]);
}

#[test]
fn g_g_comments_a_line_and_again_uncomments_it() {
    let mut ed = rust_editor("    -[f]>oo\n");

    toggle(&mut ed);
    assert_eq!(state(&ed), "    // -[f]>oo\n");

    toggle(&mut ed);
    assert_eq!(state(&ed), "    -[f]>oo\n");
}

#[test]
fn a_toggle_is_one_undo_step() {
    let mut ed = rust_editor("-[a]>\n");

    toggle(&mut ed);
    assert_eq!(state(&ed), "// -[a]>\n", "premise: the toggle edits");
    ed.handle_key(key('u'));

    assert_eq!(state(&ed), "-[a]>\n");
    assert!(!ed.doc().can_undo());
}

#[test]
fn dot_repeats_a_toggle() {
    let mut ed = rust_editor("-[a]>\nb\n");

    toggle(&mut ed);
    ed.feed_key(key('j'));
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), "// a\n// b\n");
}

#[test]
fn a_toggle_exits_extend() {
    let mut ed = rust_editor("-[a]>\n");
    ed.state.input.set_extend(true);

    toggle(&mut ed);

    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(ed.doc().text().to_string(), "// a\n");
}

#[test]
fn a_selection_over_several_lines_takes_a_block_comment() {
    let mut ed = rust_editor("-[a\nb]>\n");

    toggle(&mut ed);

    assert_eq!(state(&ed), "/* -[a\nb]> */\n");
}

#[test]
fn prefer_line_comments_gives_each_line_a_line_comment() {
    let mut ed = rust_editor("-[a\nb]>\n");
    type_cmd(&mut ed, ":set buffer comment-prefer-line=true");

    toggle(&mut ed);

    assert_eq!(state(&ed), "// -[a\n// b]>\n");
}

#[test]
fn a_read_only_buffer_refuses_the_toggle() {
    let mut ed = rust_editor("-[a]>\n");
    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only());
    let before = ed.doc().text().to_string();

    toggle(&mut ed);

    assert_eq!(ed.doc().text().to_string(), before);
    assert_eq!(status(&ed), read_only_refusal(&ed));
}

#[test]
fn a_buffer_without_a_language_refuses_the_toggle() {
    let mut ed = editor_from("-[a]>\n");

    toggle(&mut ed);

    assert_eq!(state(&ed), "-[a]>\n");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("No comment tokens: buffer has no language")
    );
}

#[test]
fn a_language_without_comment_tokens_refuses_the_toggle() {
    let mut ed = editor_in("-[a]>\n", "plain", &[], &[]);

    toggle(&mut ed);

    assert_eq!(state(&ed), "-[a]>\n");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("No comment tokens for language 'plain'")
    );
}

#[test]
fn a_span_holding_the_block_end_refuses_the_toggle_without_line_comments() {
    let mut ed = editor_in("-[a <!-- x -->\nb]>\n", "html", &[], &[("<!--", "-->")]);

    toggle(&mut ed);

    assert_eq!(state(&ed), "-[a <!-- x -->\nb]>\n");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Cannot comment: the text already holds the block comment end '-->'")
    );
}
