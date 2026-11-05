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

fn unscoped() -> BufferVisibility {
    BufferVisibility::default()
}

fn scoped(langs: &[&str]) -> BufferVisibility {
    BufferVisibility::declared(langs.iter().map(|l| l.to_string()).collect())
}

fn procs<'a>(entries: impl Iterator<Item = &'a HookEntry>) -> Vec<SteelVal> {
    entries.map(|e| e.handler.proc().value().clone()).collect()
}

fn keys(ks: &[&str]) -> Option<Box<[String]>> {
    Some(ks.iter().map(|k| k.to_string()).collect())
}

/// `register` records the body the handler was registered in.
#[test]
fn register_records_activation() {
    let mut reg = HookRegistry::default();
    reg.register(
        "on-buffer-save",
        handler(1),
        body("core:a"),
        unscoped(),
        None,
        false,
    );
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
    reg.register(
        "on-buffer-save",
        handler(1),
        body("core:a"),
        unscoped(),
        None,
        false,
    );
    reg.register(
        "on-buffer-save",
        handler(2),
        body("core:b"),
        unscoped(),
        None,
        false,
    );
    reg.register("on-buffer-save", handler(3), None, unscoped(), None, false);

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
    reg.register("on-buffer-save", handler(1), None, unscoped(), None, false);

    assert_eq!(reg.handlers_for("on-buffer-save").len(), 1);
    assert!(reg.handlers_for("on-buffer-open").is_empty());
}

/// A keyed entry matches only an event carrying one of its keys, never a
/// keyless one.
#[test]
fn keyed_entry_matches_only_its_keys() {
    let mut reg = HookRegistry::default();
    reg.register("ev", handler(1), None, unscoped(), keys(&["a", "b"]), false);

    assert_eq!(reg.matching("ev", Some("a"), Audience::Everyone).count(), 1);
    assert_eq!(reg.matching("ev", Some("b"), Audience::Everyone).count(), 1);
    assert_eq!(reg.matching("ev", Some("c"), Audience::Everyone).count(), 0);
    assert_eq!(reg.matching("ev", None, Audience::Everyone).count(), 0);
    assert!(reg.has_match("ev", Some("a"), Audience::Everyone));
    assert!(!reg.has_match("ev", Some("c"), Audience::Everyone));
}

/// An unkeyed entry matches every key and a keyless event alike, and sits
/// beside keyed entries in registration order.
#[test]
fn unkeyed_entry_matches_any_key() {
    let mut reg = HookRegistry::default();
    reg.register("ev", handler(1), None, unscoped(), keys(&["a"]), false);
    reg.register("ev", handler(2), None, unscoped(), None, false);

    let matched = |key| procs(reg.matching("ev", key, Audience::Everyone));
    assert_eq!(
        matched(Some("a")),
        vec![SteelVal::IntV(1), SteelVal::IntV(2)]
    );
    assert_eq!(matched(Some("z")), vec![SteelVal::IntV(2)]);
    assert_eq!(matched(None), vec![SteelVal::IntV(2)]);
}

/// Rollback removes a keyed entry like any other, so a failed plugin's
/// method filter stops claiming its methods.
#[test]
fn remove_registered_in_drops_keyed_entries() {
    let mut reg = HookRegistry::default();
    reg.register(
        "ev",
        handler(1),
        body("core:a"),
        unscoped(),
        keys(&["a"]),
        false,
    );

    reg.remove_registered_in(&pid("core:a"));

    assert!(!reg.has_match("ev", Some("a"), Audience::Everyone));
}

// ── Language scope ────────────────────────────────────────────────────

#[test]
fn seeing_skips_an_entry_scoped_to_another_language() {
    let mut reg = HookRegistry::default();
    reg.register(
        "ev",
        handler(1),
        body("core:a"),
        scoped(&["rust"]),
        None,
        false,
    );
    reg.register("ev", handler(2), body("core:b"), unscoped(), None, false);

    assert_eq!(
        procs(reg.matching("ev", None, Audience::Seeing(Some("markdown")))),
        vec![SteelVal::IntV(2)]
    );
    assert_eq!(
        procs(reg.matching("ev", None, Audience::Seeing(Some("rust")))),
        vec![SteelVal::IntV(1), SteelVal::IntV(2)]
    );
}

#[test]
fn everyone_reaches_every_entry() {
    let mut reg = HookRegistry::default();
    reg.register(
        "ev",
        handler(1),
        body("core:a"),
        scoped(&["rust"]),
        None,
        false,
    );
    assert_eq!(
        procs(reg.matching("ev", None, Audience::Everyone)),
        vec![SteelVal::IntV(1)]
    );
}

#[test]
fn has_match_honours_the_audience() {
    let mut reg = HookRegistry::default();
    reg.register(
        "ev",
        handler(1),
        body("core:a"),
        scoped(&["rust"]),
        None,
        false,
    );
    assert!(reg.has_match("ev", None, Audience::Seeing(Some("rust"))));
    assert!(!reg.has_match("ev", None, Audience::Seeing(None)));
}
