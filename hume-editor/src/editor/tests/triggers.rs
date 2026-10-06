// Trigger characters: language and attachment scopes, hook and completion
// kinds, and the completion sources a typed character invokes.

use super::*;
use crate::editor::completion::{BufferSourceEntry, MatchKind};
use hume_scripting::TriggerKind::{Completion, Hook};
use hume_scripting::TriggerScope;
use steel::rvals::SteelVal;

fn source(name: &str) -> BufferSourceEntry {
    BufferSourceEntry {
        name: name.into(),
        match_kind: MatchKind::Fuzzy,
        priority: 0,
        proc: SteelVal::Void,
        resolve: false,
        token_chars: "".into(),
    }
}

fn language(name: &str) -> TriggerScope {
    TriggerScope::Language(name.to_string())
}

/// A buffer whose language is `rust`.
fn rust_buffer() -> (crate::editor::Editor, BufferId) {
    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let rust = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(rust));
    (ed, bid)
}

#[test]
fn a_completion_set_for_an_unknown_source_errors() {
    let (mut ed, _) = rust_buffer();
    let err = ed
        .state
        .set_triggers(Completion, "nope".into(), language("rust"), vec!['.'])
        .expect_err("no buffer source named \"nope\"");
    assert!(
        err.contains("no buffer completion source named"),
        "got: {err}"
    );
}

#[test]
fn a_language_set_joins_the_registered_source() {
    let (mut ed, bid) = rust_buffer();
    ed.state
        .config
        .completion_sources
        .register_buffer(source("dot"));
    assert!(ed.state.completion_sources_for_trigger('.', bid).is_empty());

    ed.state
        .set_triggers(Completion, "dot".into(), language("rust"), vec!['.'])
        .unwrap();
    let id = ed.state.config.completion_sources.buffer_id_of("dot");
    assert_eq!(
        ed.state.completion_sources_for_trigger('.', bid),
        id.into_iter().collect::<Vec<_>>()
    );
    assert!(ed.state.completion_sources_for_trigger(':', bid).is_empty());

    ed.state
        .set_triggers(Completion, "dot".into(), language("python"), vec![':'])
        .unwrap();
    assert!(
        ed.state.completion_sources_for_trigger(':', bid).is_empty(),
        "another language's set does not apply to a rust buffer"
    );
}

#[test]
fn an_empty_set_clears_the_entry() {
    let (mut ed, bid) = rust_buffer();
    ed.state
        .config
        .completion_sources
        .register_buffer(source("dot"));
    ed.state
        .set_triggers(Completion, "dot".into(), language("rust"), vec!['.'])
        .unwrap();
    ed.state
        .set_triggers(Completion, "dot".into(), language("rust"), Vec::new())
        .unwrap();
    assert!(ed.state.completion_sources_for_trigger('.', bid).is_empty());
}

#[test]
fn re_registering_a_source_keeps_its_triggers() {
    let (mut ed, bid) = rust_buffer();
    let sources = &mut ed.state.config.completion_sources;
    sources.register_buffer(source("dot"));
    ed.state
        .set_triggers(Completion, "dot".into(), language("rust"), vec!['.'])
        .unwrap();
    ed.state
        .config
        .completion_sources
        .register_buffer(source("dot"));
    assert_eq!(ed.state.completion_sources_for_trigger('.', bid).len(), 1);
}

#[test]
fn sources_come_back_in_registration_order() {
    let (mut ed, bid) = rust_buffer();
    for name in ["first", "second"] {
        ed.state
            .config
            .completion_sources
            .register_buffer(source(name));
    }
    ed.state
        .set_triggers(Completion, "second".into(), language("rust"), vec!['.'])
        .unwrap();
    ed.state
        .set_triggers(Completion, "first".into(), language("rust"), vec!['.'])
        .unwrap();
    let names: Vec<String> = ed
        .state
        .completion_sources_for_trigger('.', bid)
        .into_iter()
        .map(|id| {
            ed.state
                .config
                .completion_sources
                .buffer_get(id)
                .name
                .to_string()
        })
        .collect();
    assert_eq!(names, ["first", "second"]);
}

#[test]
fn hook_and_completion_sets_of_one_source_are_independent() {
    let (mut ed, bid) = rust_buffer();
    ed.state
        .config
        .completion_sources
        .register_buffer(source("both"));
    ed.state
        .set_triggers(Hook, "both".into(), language("rust"), vec!['('])
        .unwrap();
    ed.state
        .set_triggers(Completion, "both".into(), language("rust"), vec!['.'])
        .unwrap();

    assert_eq!(ed.state.trigger_sources_for(Hook, '(', bid), ["both"]);
    assert!(ed.state.trigger_sources_for(Hook, '.', bid).is_empty());
    assert_eq!(ed.state.trigger_sources_for(Completion, '.', bid), ["both"]);
    assert!(
        ed.state
            .trigger_sources_for(Completion, '(', bid)
            .is_empty()
    );
}

#[test]
fn a_hook_set_needs_no_registered_source() {
    let (mut ed, bid) = rust_buffer();
    ed.state
        .set_triggers(Hook, "listener".into(), language("rust"), vec!['('])
        .unwrap();
    assert_eq!(ed.state.trigger_sources_for(Hook, '(', bid), ["listener"]);
}
