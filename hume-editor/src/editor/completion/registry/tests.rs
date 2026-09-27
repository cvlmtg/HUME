use super::*;

fn steel_buffer_entry(name: &str) -> BufferSourceEntry {
    BufferSourceEntry {
        name: name.into(),
        match_kind: MatchKind::Fuzzy,
        priority: 0,
        proc: SteelVal::Void,
        resolve: false,
        trigger_chars: rustc_hash::FxHashMap::default(),
    }
}

fn steel_minibuf_entry(name: &str) -> MinibufSourceEntry {
    MinibufSourceEntry {
        name: name.into(),
        match_kind: MatchKind::Fuzzy,
        priority: 0,
        body: MinibufBody::Steel(SteelVal::Void),
    }
}

#[test]
fn register_buffer_under_a_fresh_name_adds() {
    let mut reg = SourceRegistry::with_defaults();
    let outcome = reg.register_buffer(steel_buffer_entry("my-source"));
    assert!(matches!(outcome, RegisterOutcome::Added));
    assert_eq!(reg.buffer.len(), 1);
}

#[test]
fn register_minibuf_under_a_same_name_replaces_in_place() {
    let mut reg = SourceRegistry::with_defaults();
    let id_before = reg
        .minibuf_id_of("path")
        .expect("native path source registered");
    let outcome = reg.register_minibuf(steel_minibuf_entry("path"));
    assert!(matches!(outcome, RegisterOutcome::Replaced));
    let id_after = reg.minibuf_id_of("path").expect("still registered");
    assert_eq!(
        id_before, id_after,
        "MinibufSourceId must stay stable across a replace"
    );
    assert!(matches!(
        reg.minibuf_get(id_after).body,
        MinibufBody::Steel(_)
    ));
}

/// A name taken in one namespace is simply unrelated to the same name in
/// the other: `register_buffer`/`register_minibuf` each only ever search
/// their own `Vec`, so there is no cross-namespace collision to refuse.
#[test]
fn the_same_name_in_both_namespaces_registers_two_independent_sources() {
    let mut reg = SourceRegistry::with_defaults();
    let native_path_id = reg
        .minibuf_id_of("path")
        .expect("native path source registered");
    let outcome = reg.register_buffer(steel_buffer_entry("path"));
    assert!(matches!(outcome, RegisterOutcome::Added));
    let buffer_path_id = reg.buffer_id_of("path").expect("just registered");

    // The native minibuf "path" source is untouched.
    assert_eq!(reg.minibuf_id_of("path"), Some(native_path_id));
    assert!(matches!(
        reg.minibuf_get(native_path_id).body,
        MinibufBody::NativeDelegated(_)
    ));
    // The new buffer "path" source is its own, separate entry.
    assert_eq!(reg.buffer_get(buffer_path_id).name.as_ref(), "path");
}

// ── Trigger chars ────────────────────────────────────────────────────────────

#[test]
fn set_buffer_trigger_chars_on_an_unknown_name_errors() {
    let mut reg = SourceRegistry::with_defaults();
    let err = reg
        .set_buffer_trigger_chars("nope", "rust".to_string(), vec!['.'])
        .expect_err("no buffer source named \"nope\"");
    assert!(
        err.contains("no buffer completion source named"),
        "got: {err}"
    );
}

#[test]
fn set_buffer_trigger_chars_joins_the_registered_source() {
    let mut reg = SourceRegistry::with_defaults();
    reg.register_buffer(steel_buffer_entry("dot"));
    let id = reg.buffer_id_of("dot").expect("just registered");
    assert!(
        reg.buffer_sources_for_trigger('.', Some("rust")).is_empty(),
        "nothing registered yet"
    );
    reg.set_buffer_trigger_chars("dot", "rust".to_string(), vec!['.'])
        .expect("known name");
    assert_eq!(reg.buffer_sources_for_trigger('.', Some("rust")), vec![id]);
    // A different language's own entry is unaffected.
    assert!(
        reg.buffer_sources_for_trigger('.', Some("python"))
            .is_empty()
    );
}

#[test]
fn set_buffer_trigger_chars_empty_clears_the_entry() {
    let mut reg = SourceRegistry::with_defaults();
    reg.register_buffer(steel_buffer_entry("dot"));
    reg.set_buffer_trigger_chars("dot", "rust".to_string(), vec!['.'])
        .expect("known name");
    reg.set_buffer_trigger_chars("dot", "rust".to_string(), Vec::new())
        .expect("known name");
    assert!(reg.buffer_sources_for_trigger('.', Some("rust")).is_empty());
}

/// A same-name re-registration (`register_buffer` replacing the entry in
/// place) must not wipe trigger chars set separately:
/// `set_buffer_trigger_chars` and `register_buffer` write different facts
/// at different times.
#[test]
fn re_registering_a_buffer_source_preserves_its_trigger_chars() {
    let mut reg = SourceRegistry::with_defaults();
    reg.register_buffer(steel_buffer_entry("dot"));
    reg.set_buffer_trigger_chars("dot", "rust".to_string(), vec!['.'])
        .expect("known name");
    let id_before = reg.buffer_id_of("dot").expect("just registered");
    reg.register_buffer(steel_buffer_entry("dot"));
    let id_after = reg.buffer_id_of("dot").expect("still registered");
    assert_eq!(id_before, id_after, "id stays stable across a replace");
    assert_eq!(
        reg.buffer_sources_for_trigger('.', Some("rust")),
        vec![id_after]
    );
}
