use super::*;

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
    reg.register("on-buffer-save", Some(pid("core:a")), SteelVal::IntV(1));
    assert_eq!(
        reg.handlers_for("on-buffer-save")[0].owner,
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
    reg.register("on-buffer-save", Some(pid("core:a")), SteelVal::IntV(1));
    reg.register("on-buffer-save", Some(pid("core:b")), SteelVal::IntV(2));
    reg.register("on-buffer-save", None, SteelVal::IntV(3));

    reg.remove_owned_by(&pid("core:a"));

    let survivors = reg.handlers_for("on-buffer-save");
    assert_eq!(survivors.len(), 2, "only core:a's entry must be removed");
    assert_eq!(survivors[0].owner, Some(pid("core:b")));
    assert_eq!(survivors[1].owner, None);
}

/// A handler registered for one hook name is not returned for another —
/// pins the name-keyed map against key collisions.
///
/// Fail oracle: a hash/eq bug (or hardcoding a lookup key) that maps two
/// distinct names to the same bucket would leak `on-buffer-save`'s handler
/// into `on-buffer-open`'s list.
#[test]
fn handlers_are_isolated_per_name() {
    let mut reg = HookRegistry::default();
    reg.register("on-buffer-save", None, SteelVal::IntV(1));

    assert_eq!(reg.handlers_for("on-buffer-save").len(), 1);
    assert!(reg.handlers_for("on-buffer-open").is_empty());
}
