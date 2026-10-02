use super::*;
use crate::attribution::PluginId;

fn pid(s: &str) -> EntryId {
    EntryId::main(PluginId::parse(s).unwrap())
}

fn keys(ks: &[&str]) -> Option<Box<[String]>> {
    Some(ks.iter().map(|k| k.to_string()).collect())
}

/// `register` records the given owner on the stored entry.
#[test]
fn register_records_owner() {
    let mut reg = HookRegistry::default();
    reg.register(
        "on-buffer-save",
        Some(pid("core:a")),
        SteelVal::IntV(1),
        None,
    );
    assert_eq!(
        reg.handlers_for("on-buffer-save")[0].owner,
        Some(pid("core:a"))
    );
}

/// `remove_owned_by` removes only entries owned by the given plugin,
/// leaving other owners (including `None`, top-level registrations)
/// untouched.
#[test]
fn remove_owned_by_removes_only_matching_owner() {
    let mut reg = HookRegistry::default();
    reg.register(
        "on-buffer-save",
        Some(pid("core:a")),
        SteelVal::IntV(1),
        None,
    );
    reg.register(
        "on-buffer-save",
        Some(pid("core:b")),
        SteelVal::IntV(2),
        None,
    );
    reg.register("on-buffer-save", None, SteelVal::IntV(3), None);

    reg.remove_owned_by(&pid("core:a"));

    let survivors = reg.handlers_for("on-buffer-save");
    assert_eq!(survivors.len(), 2, "only core:a's entry must be removed");
    assert_eq!(survivors[0].owner, Some(pid("core:b")));
    assert_eq!(survivors[1].owner, None);
}

/// A handler registered for one hook name is not returned for another.
/// Pins the name-keyed map against key collisions.
///
/// A hash or equality bug that mapped two distinct names to the same bucket
/// would leak `on-buffer-save`'s handler into `on-buffer-open`'s list.
#[test]
fn handlers_are_isolated_per_name() {
    let mut reg = HookRegistry::default();
    reg.register("on-buffer-save", None, SteelVal::IntV(1), None);

    assert_eq!(reg.handlers_for("on-buffer-save").len(), 1);
    assert!(reg.handlers_for("on-buffer-open").is_empty());
}

/// A keyed entry matches only an event carrying one of its keys, never a
/// keyless one.
#[test]
fn keyed_entry_matches_only_its_keys() {
    let mut reg = HookRegistry::default();
    reg.register("ev", None, SteelVal::IntV(1), keys(&["a", "b"]));

    assert_eq!(reg.matching("ev", Some("a")).count(), 1);
    assert_eq!(reg.matching("ev", Some("b")).count(), 1);
    assert_eq!(reg.matching("ev", Some("c")).count(), 0);
    assert_eq!(reg.matching("ev", None).count(), 0);
    assert!(reg.has_match("ev", Some("a")));
    assert!(!reg.has_match("ev", Some("c")));
}

/// An unkeyed entry matches every key and a keyless event alike, and sits
/// beside keyed entries in registration order.
#[test]
fn unkeyed_entry_matches_any_key() {
    let mut reg = HookRegistry::default();
    reg.register("ev", None, SteelVal::IntV(1), keys(&["a"]));
    reg.register("ev", None, SteelVal::IntV(2), None);

    let procs = |key| {
        reg.matching("ev", key)
            .map(|e| e.proc.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(procs(Some("a")), vec![SteelVal::IntV(1), SteelVal::IntV(2)]);
    assert_eq!(procs(Some("z")), vec![SteelVal::IntV(2)]);
    assert_eq!(procs(None), vec![SteelVal::IntV(2)]);
}

/// Rollback removes a keyed entry like any other, so a failed plugin's
/// method filter stops claiming its methods.
#[test]
fn remove_owned_by_drops_keyed_entries() {
    let mut reg = HookRegistry::default();
    reg.register("ev", Some(pid("core:a")), SteelVal::IntV(1), keys(&["a"]));

    reg.remove_owned_by(&pid("core:a"));

    assert!(!reg.has_match("ev", Some("a")));
}
