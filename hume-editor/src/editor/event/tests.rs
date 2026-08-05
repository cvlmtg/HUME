use super::*;

/// Never called — its only purpose is the exhaustive `match`: adding an
/// `EditorEvent` variant without extending this list is a compile error, not
/// a runtime `.expect()` panic the first time something raises the new
/// event. Keep in lockstep with `ALL_VARIANTS` below.
#[allow(dead_code)]
fn _exhaustiveness_check(event: EditorEvent) {
    match event {
        EditorEvent::OnBufferOpen
        | EditorEvent::OnBufferClose
        | EditorEvent::OnBufferSave
        | EditorEvent::OnModeChange
        | EditorEvent::OnLanguageSet
        | EditorEvent::OnLspAttach
        | EditorEvent::OnLspDetach
        | EditorEvent::OnDiagnosticsChanged
        | EditorEvent::OnViewportChange
        | EditorEvent::OnTriggerChar
        | EditorEvent::OnCompletionAccept
        | EditorEvent::OnCompletionRefilter => {}
    }
}

const ALL_VARIANTS: &[EditorEvent] = &[
    EditorEvent::OnBufferOpen,
    EditorEvent::OnBufferClose,
    EditorEvent::OnBufferSave,
    EditorEvent::OnModeChange,
    EditorEvent::OnLanguageSet,
    EditorEvent::OnLspAttach,
    EditorEvent::OnLspDetach,
    EditorEvent::OnDiagnosticsChanged,
    EditorEvent::OnViewportChange,
    EditorEvent::OnTriggerChar,
    EditorEvent::OnCompletionAccept,
    EditorEvent::OnCompletionRefilter,
];

/// Every `ALL_VARIANTS` entry has a table row, and that row maps back to the
/// same variant — i.e. `EDITOR_EVENT_NAMES` is not missing or mispairing any
/// of today's (all Steel-visible) variants.
///
/// Fail oracle: delete an `EDITOR_EVENT_NAMES` row for a variant still in
/// `ALL_VARIANTS` → `name()` returns `None`, caught here instead of at
/// runtime the first time the event fires. Swap two rows' names → the
/// reverse lookup lands on the wrong variant.
#[test]
fn every_variant_round_trips_through_name_and_the_table() {
    for &event in ALL_VARIANTS {
        let name = event.name().expect("variant must have a table entry");
        let (found_event, _) = EDITOR_EVENT_NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .expect("name must resolve back through the table");
        assert_eq!(*found_event, event, "round trip failed for {name}");
    }
}

#[test]
fn known_event_names_has_no_duplicates_and_matches_variant_count() {
    let names = known_event_names();
    assert_eq!(names.len(), ALL_VARIANTS.len());
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "EDITOR_EVENT_NAMES has a duplicate symbol name"
    );
}
