use super::*;

/// Never called — its only purpose is the exhaustive `match`: adding a
/// `HookId` variant without extending this list is a compile error, not
/// a runtime `.expect()` panic the first time something fires the new
/// hook. Keep in lockstep with `ALL_VARIANTS` below.
#[allow(dead_code)]
fn _exhaustiveness_check(id: HookId) {
    match id {
        HookId::OnBufferOpen
        | HookId::OnBufferClose
        | HookId::OnBufferSave
        | HookId::OnModeChange
        | HookId::OnLanguageSet
        | HookId::OnLspAttach
        | HookId::OnLspDetach
        | HookId::OnDiagnosticsChanged
        | HookId::OnViewportChange
        | HookId::OnTriggerChar
        | HookId::OnCompletionAccept
        | HookId::OnCompletionRefilter => {}
    }
}

const ALL_VARIANTS: &[HookId] = &[
    HookId::OnBufferOpen,
    HookId::OnBufferClose,
    HookId::OnBufferSave,
    HookId::OnModeChange,
    HookId::OnLanguageSet,
    HookId::OnLspAttach,
    HookId::OnLspDetach,
    HookId::OnDiagnosticsChanged,
    HookId::OnViewportChange,
    HookId::OnTriggerChar,
    HookId::OnCompletionAccept,
    HookId::OnCompletionRefilter,
];

/// Fail oracle: delete a HOOKS row for a variant still in `ALL_VARIANTS`
/// → `symbol()` panics (caught as a normal test failure here, not a
/// runtime surprise the first time the hook fires).
#[test]
fn every_hook_id_round_trips_through_symbol_and_from_symbol() {
    for &id in ALL_VARIANTS {
        let name = id.symbol();
        assert_eq!(
            HookId::from_symbol(name),
            Some(id),
            "round trip failed for {name}"
        );
    }
}

#[test]
fn all_names_has_no_duplicates_and_matches_variant_count() {
    let names: Vec<&str> = HookId::all_names().collect();
    assert_eq!(names.len(), ALL_VARIANTS.len());
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "HOOKS has a duplicate symbol name"
    );
}

// ── Owner tracking / rollback ─────────────────────────────────────────────

fn pid(s: &str) -> PluginId {
    PluginId::parse(s).unwrap()
}

/// `register` records the given owner on the stored entry.
///
/// Fail oracle: if `register` dropped `owner` on the floor, this would
/// read back `None` regardless of what was passed in.
#[test]
fn register_records_owner() {
    let mut reg = HookRegistry::default();
    reg.register(HookId::OnBufferSave, Some(pid("core:a")), SteelVal::IntV(1));
    assert_eq!(
        reg.handlers_for(HookId::OnBufferSave)[0].owner,
        Some(pid("core:a"))
    );
}

/// `remove_owned_by` removes only entries owned by the given plugin,
/// leaving other owners (including `None`, top-level registrations)
/// untouched.
///
/// Fail oracle: revert `remove_owned_by` to a no-op → all three entries
/// survive → the length assert fires.
#[test]
fn remove_owned_by_removes_only_matching_owner() {
    let mut reg = HookRegistry::default();
    reg.register(HookId::OnBufferSave, Some(pid("core:a")), SteelVal::IntV(1));
    reg.register(HookId::OnBufferSave, Some(pid("core:b")), SteelVal::IntV(2));
    reg.register(HookId::OnBufferSave, None, SteelVal::IntV(3));

    reg.remove_owned_by(&pid("core:a"));

    let survivors = reg.handlers_for(HookId::OnBufferSave);
    assert_eq!(survivors.len(), 2, "only core:a's entry must be removed");
    assert_eq!(survivors[0].owner, Some(pid("core:b")));
    assert_eq!(survivors[1].owner, None);
}
