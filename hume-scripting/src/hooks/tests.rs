use super::*;
use crate::attribution::PluginId;
use steel::rvals::SteelVal;

fn pid(s: &str) -> EntryId {
    EntryId::main(PluginId::parse(s).unwrap())
}

/// A handler whose proc is the integer `n`.
fn handler(n: isize) -> Callback {
    Callback::for_test(SteelVal::IntV(n))
}

/// The body `id` names, as the activation a registration belongs to.
fn body(id: &str) -> Option<Rc<EntryId>> {
    Some(Rc::new(pid(id)))
}

fn keys(ks: &[&str]) -> Option<Box<[String]>> {
    Some(ks.iter().map(|k| k.to_string()).collect())
}

/// `register` records the body the handler was registered in.
#[test]
fn register_records_activation() {
    let mut reg = HookRegistry::default();
    reg.register("on-buffer-save", handler(1), body("core:a"), None, false);
    assert_eq!(
        reg.handlers_for("on-buffer-save")[0].activation.as_deref(),
        Some(&pid("core:a"))
    );
}

/// `remove_registered_in` removes only the entries registered in that
/// body, leaving another body's and those registered outside any body.
#[test]
fn remove_registered_in_removes_only_that_bodys_entries() {
    let mut reg = HookRegistry::default();
    reg.register("on-buffer-save", handler(1), body("core:a"), None, false);
    reg.register("on-buffer-save", handler(2), body("core:b"), None, false);
    reg.register("on-buffer-save", handler(3), None, None, false);

    reg.remove_registered_in(&pid("core:a"));

    let survivors: Vec<SteelVal> = reg
        .handlers_for("on-buffer-save")
        .iter()
        .map(|e| e.handler.proc().value().clone())
        .collect();
    assert_eq!(survivors, vec![SteelVal::IntV(2), SteelVal::IntV(3)]);
}

/// A handler registered for one hook name is not returned for another.
/// Pins the name-keyed map against key collisions.
///
/// A hash or equality bug that mapped two distinct names to the same bucket
/// would leak `on-buffer-save`'s handler into `on-buffer-open`'s list.
#[test]
fn handlers_are_isolated_per_name() {
    let mut reg = HookRegistry::default();
    reg.register("on-buffer-save", handler(1), None, None, false);

    assert_eq!(reg.handlers_for("on-buffer-save").len(), 1);
    assert!(reg.handlers_for("on-buffer-open").is_empty());
}

/// A keyed entry matches only an event carrying one of its keys, never a
/// keyless one.
#[test]
fn keyed_entry_matches_only_its_keys() {
    let mut reg = HookRegistry::default();
    reg.register("ev", handler(1), None, keys(&["a", "b"]), false);

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
    reg.register("ev", handler(1), None, keys(&["a"]), false);
    reg.register("ev", handler(2), None, None, false);

    let procs = |key| {
        reg.matching("ev", key)
            .map(|e| e.handler.proc().value().clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(procs(Some("a")), vec![SteelVal::IntV(1), SteelVal::IntV(2)]);
    assert_eq!(procs(Some("z")), vec![SteelVal::IntV(2)]);
    assert_eq!(procs(None), vec![SteelVal::IntV(2)]);
}

/// Rollback removes a keyed entry like any other, so a failed plugin's
/// method filter stops claiming its methods.
#[test]
fn remove_registered_in_drops_keyed_entries() {
    let mut reg = HookRegistry::default();
    reg.register("ev", handler(1), body("core:a"), keys(&["a"]), false);

    reg.remove_registered_in(&pid("core:a"));

    assert!(!reg.has_match("ev", Some("a")));
}
