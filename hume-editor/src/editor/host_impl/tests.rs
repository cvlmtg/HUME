use std::path::Path;

use hume_engine::pipeline::BufferId;
use hume_scripting::host::{BufferHost, LanguageHost};

use crate::editor::Editor;
use crate::editor::scripting_setup::make_init_host;

#[test]
fn close_buffer_errs_when_id_unknown() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::Text::empty(),
        hume_editing::selection::SelectionSet::default(),
    ));
    let mut host = make_init_host(&mut ed.state, &mut ed.view);
    // BufferId::default() is a zeroed key — not present in any live store.
    let err = host.close_buffer(BufferId::default()).unwrap_err();
    assert!(!err.is_empty(), "expected an error message");
}

#[test]
fn switch_to_buffer_noop_when_same() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::Text::empty(),
        hume_editing::selection::SelectionSet::default(),
    ));
    let bid = ed.focused_buffer_id();
    let mut host = make_init_host(&mut ed.state, &mut ed.view);
    // Switching to the same buffer should not error.
    host.switch_to_buffer(bid, bid).expect("same-buffer switch");
}

#[test]
fn attach_grammar_errs_for_bad_path() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::Text::empty(),
        hume_editing::selection::SelectionSet::default(),
    ));
    let mut host = make_init_host(&mut ed.state, &mut ed.view);
    let err = host
        .attach_grammar(
            "rust",
            Path::new("/no/such/lib.dylib"),
            "rust_language",
            Path::new("/no/such/highlights.scm"),
            None,
        )
        .unwrap_err();
    assert!(
        err.contains("register-grammar!"),
        "unexpected message: {err}"
    );
}
