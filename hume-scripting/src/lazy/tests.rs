use super::*;

fn fake_path() -> PathBuf {
    PathBuf::from("/fake/plugin.scm")
}

fn id_core(name: &str) -> EntryId {
    EntryId::main(PluginId::Core(name.to_string()))
}

fn id_user(user: &str, repo: &str) -> EntryId {
    EntryId::main(PluginId::User {
        user: user.to_string(),
        repo: repo.to_string(),
    })
}

// ── PluginState inserted for resolved plugins ─────────────────────────

#[test]
fn declare_resolved_path_inserts_declared_state() {
    let mut reg = LazyRegistry::default();
    let id = id_core("plum");
    reg.declare(id.clone(), fake_path(), vec![], vec![]);
    assert!(
        matches!(reg.plugins[&id], PluginState::Declared { .. }),
        "resolved plugin must be Declared"
    );
}

// ── One declaration per entry ─────────────────────────────────────────

#[test]
fn an_entry_is_found_whatever_its_casing() {
    let mut reg = LazyRegistry::default();
    reg.declare(id_user("Alice", "Foo"), fake_path(), vec![], vec![]);
    assert!(reg.plugins.contains_key(&id_user("alice", "foo")));
}

/// `declare_plugin` raises for a second declaration, so one reaching the
/// registry is a broken caller.
#[test]
#[should_panic(expected = "declared twice")]
fn declaring_an_entry_twice_panics() {
    let mut reg = LazyRegistry::default();
    reg.declare(id_user("Alice", "Foo"), fake_path(), vec![], vec![]);
    reg.declare(id_user("alice", "foo"), fake_path(), vec![], vec![]);
}

// ── Activation map recording ───────────────────────────────────────────

#[test]
fn event_plugins_lists_the_entries_declared_for_that_event_only() {
    let mut reg = LazyRegistry::default();
    let a = id_user("a", "x");
    reg.declare(
        a.clone(),
        fake_path(),
        vec!["on-buffer-save".to_string()],
        vec![],
    );

    assert_eq!(reg.event_plugins("on-buffer-save"), [a]);
    assert!(reg.event_plugins("on-buffer-open").is_empty());
}

#[test]
fn activation_events_are_one_to_many() {
    let mut reg = LazyRegistry::default();
    let a = id_user("a", "x");
    let b = id_user("b", "y");
    reg.declare(
        a.clone(),
        fake_path(),
        vec!["on-buffer-save".to_string()],
        vec![],
    );
    reg.declare(
        b.clone(),
        fake_path(),
        vec!["on-buffer-save".to_string()],
        vec![],
    );
    let handlers = &reg.activation_events["on-buffer-save"];
    assert_eq!(
        handlers.len(),
        2,
        "two plugins must both register for the hook"
    );
    assert!(handlers.contains(&a));
    assert!(handlers.contains(&b));
}

#[test]
fn activation_languages_are_one_to_many() {
    let mut reg = LazyRegistry::default();
    let a = id_user("a", "x");
    let b = id_user("b", "y");
    reg.declare(a.clone(), fake_path(), vec![], vec!["rust".to_string()]);
    reg.declare(b.clone(), fake_path(), vec![], vec!["rust".to_string()]);
    let handlers = &reg.activation_languages["rust"];
    assert_eq!(handlers.len(), 2);
    assert!(handlers.contains(&a));
    assert!(handlers.contains(&b));
}

#[test]
fn multiple_events_for_one_plugin() {
    let mut reg = LazyRegistry::default();
    let id = id_user("user", "repo");
    reg.declare(
        id.clone(),
        fake_path(),
        vec!["on-buffer-open".to_string(), "on-buffer-save".to_string()],
        vec![],
    );
    assert!(reg.activation_events["on-buffer-open"].contains(&id));
    assert!(reg.activation_events["on-buffer-save"].contains(&id));
}

// ── Loaded-plugins derivation (used by (loaded-plugins) builtin) ──────

#[test]
fn loaded_plugins_derived_from_state() {
    let mut reg = LazyRegistry::default();
    let loaded_id = id_user("loaded", "one");
    let declared_id = id_user("pending", "two");

    reg.declare(loaded_id.clone(), fake_path(), vec![], vec![]);
    reg.declare(declared_id.clone(), fake_path(), vec![], vec![]);

    // Manually advance one to Loaded to simulate finish_lazy_activation.
    *reg.plugins.get_mut(&loaded_id).unwrap() = PluginState::Loaded;

    let loaded: Vec<_> = reg
        .plugins
        .iter()
        .filter(|(_, s)| matches!(s, PluginState::Loaded))
        .map(|(id, _)| id.to_string())
        .collect();

    assert_eq!(loaded, vec!["loaded/one"]);
}

// ── format_status ─────────────────────────────────────────────────────

#[test]
fn format_status_empty_returns_empty_string() {
    let reg = LazyRegistry::default();
    assert_eq!(
        reg.format_status([], &[]),
        "",
        "empty registry must return empty"
    );
}

#[test]
fn format_status_waiting_with_triggers() {
    let mut reg = LazyRegistry::default();
    let id = id_user("alice", "lazy");
    reg.declare(
        id.clone(),
        fake_path(),
        vec!["on-buffer-save".to_string()],
        vec!["rust".to_string()],
    );
    let lazy_cmds = vec![("my-cmd".to_string(), id, false)];
    let out = reg.format_status([], &lazy_cmds);
    assert!(out.contains("alice/lazy"), "plugin id must appear");
    assert!(out.contains("declared"), "state must be 'declared'");
    assert!(
        out.contains("cmd:my-cmd"),
        "command activation entry must appear"
    );
    assert!(
        out.contains("event:on-buffer-save"),
        "event activation entry must appear"
    );
    assert!(
        out.contains("lang:rust"),
        "language activation entry must appear"
    );
}

// The data layer accepts zero-activation plugins (LazyRegistry::declare has no
// policy gate); the policy guard lives in declare_plugin (builtins layer).
// This test exercises the defensive placeholder fallback in
// pending_activations. Which glyph is used is an appearance detail, not
// asserted here.
#[test]
fn format_status_zero_trigger_shows_a_placeholder() {
    let mut reg = LazyRegistry::default();
    reg.declare(id_user("bob", "bare"), fake_path(), vec![], vec![]);
    let out = reg.format_status([], &[]);
    let row = out
        .lines()
        .find(|l| l.contains("bob/bare"))
        .expect("row for bob/bare must exist");
    let fields: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(
        fields.len(),
        3,
        "zero-trigger plugin must show a placeholder in the activations column, got {row:?}"
    );
    assert_eq!(fields[1], "declared");
    assert!(!row.contains("cmd:"), "no cmd prefix for zero-entry plugin");
}

#[test]
fn format_status_loaded_shows_no_triggers() {
    let mut reg = LazyRegistry::default();
    let id = id_user("carol", "eager");
    reg.declare(id.clone(), fake_path(), vec![], vec![]);
    // Simulate finish_lazy_activation: set Loaded. The editor's Lazy stub for
    // "eager-cmd" is gone by now too (unregister_lazy_stubs_of already
    // ran), so the caller passes an empty lazy_cmds, exactly what a real
    // post-activation `:plugin-status` call would see.
    *reg.plugins.get_mut(&id).unwrap() = PluginState::Loaded;

    let out = reg.format_status([], &[]);
    assert!(out.contains("carol/eager"), "plugin id must appear");
    assert!(out.contains("loaded"), "state must be 'loaded'");
    assert!(
        !out.contains("eager-cmd"),
        "loaded plugin must not show its old activation entry"
    );
}

#[test]
fn format_status_failed_shows_no_triggers() {
    let mut reg = LazyRegistry::default();
    let id = id_core("broken");
    reg.declare(id.clone(), fake_path(), vec![], vec![]);
    *reg.plugins.get_mut(&id).unwrap() = PluginState::Failed;

    let out = reg.format_status([], &[]);
    assert!(out.contains("core:broken"));
    assert!(out.contains("failed"));
    assert!(
        !out.contains("broken-cmd"),
        "failed plugin must not show activation entry"
    );
}

#[test]
fn format_status_sorts_by_id() {
    let mut reg = LazyRegistry::default();
    reg.declare(id_user("z", "last"), fake_path(), vec![], vec![]);
    reg.declare(id_user("a", "first"), fake_path(), vec![], vec![]);
    let out = reg.format_status([], &[]);
    let z_pos = out.find("z/last").expect("z/last must appear");
    let a_pos = out.find("a/first").expect("a/first must appear");
    assert!(a_pos < z_pos, "rows must be sorted alphabetically by id");
}

#[test]
fn format_status_lists_a_plugin_without_entries_on_its_own_row() {
    let mut reg = LazyRegistry::default();
    reg.declare(id_user("m", "present"), fake_path(), vec![], vec![]);
    let gone = PluginId::parse("z/gone").unwrap();
    let broken = PluginId::parse("a/broken").unwrap();
    let has_entries = PluginId::parse("m/present").unwrap();

    let out = reg.format_status(
        [
            (&gone, "absent"),
            (&broken, "failed"),
            (&has_entries, "failed"),
        ],
        &[],
    );
    let rows: Vec<(&str, &str)> = out
        .lines()
        .skip(1)
        .map(|l| {
            let mut cols = l.split_whitespace();
            (cols.next().unwrap(), cols.next().unwrap())
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("a/broken", "failed"),
            ("m/present", "declared"),
            ("z/gone", "absent")
        ]
    );
}

#[test]
fn language_plugins_lists_the_language_then_the_wildcard_each_once() {
    let mut reg = LazyRegistry::default();
    let (rust, both, any) = (id_core("rust"), id_core("both"), id_core("any"));
    reg.declare(rust.clone(), fake_path(), vec![], vec!["rust".to_string()]);
    reg.declare(
        both.clone(),
        fake_path(),
        vec![],
        vec!["rust".to_string(), "*".to_string()],
    );
    reg.declare(any.clone(), fake_path(), vec![], vec!["*".to_string()]);

    assert_eq!(
        reg.language_plugins("rust"),
        [rust, both.clone(), any.clone()]
    );
    assert_eq!(reg.language_plugins("go"), [both, any]);
}
