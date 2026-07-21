use super::*;

/// Convenience: parse a known-valid plugin name in tests.
fn pid(s: &str) -> PluginId {
    PluginId::parse(s).expect("valid plugin id in test")
}

// ── PluginId::parse ───────────────────────────────────────────────────────

#[test]
fn parse_core_plugin() {
    let id = PluginId::parse("core:plum").unwrap();
    assert!(matches!(id, PluginId::Core(ref n) if n == "plum"));
    assert_eq!(id.to_string(), "core:plum");
}

#[test]
fn parse_user_plugin() {
    let id = PluginId::parse("alice/bar").unwrap();
    assert!(matches!(&id, PluginId::User { user, repo } if user == "alice" && repo == "bar"));
    assert_eq!(id.to_string(), "alice/bar");
}

#[test]
fn parse_just_a_name_errors() {
    assert!(PluginId::parse("just-a-name").is_err());
}

#[test]
fn parse_dotdot_segment_errors() {
    assert!(PluginId::parse("../evil").is_err());
    assert!(PluginId::parse("core:../evil").is_err()); // dotdot core name
    assert!(PluginId::parse("core:..").is_err());
    assert!(PluginId::parse("../evil").is_err());
}

#[test]
fn parse_empty_segment_errors() {
    assert!(PluginId::parse("core:").is_err());
    assert!(PluginId::parse("/repo").is_err());
    assert!(PluginId::parse("user/").is_err());
}

#[test]
fn parse_too_many_slashes_errors() {
    assert!(PluginId::parse("a/b/c").is_err());
}

#[test]
fn parse_quote_in_segment_errors() {
    assert!(PluginId::parse("core:a\"b").is_err());
    assert!(PluginId::parse("a\"b/repo").is_err());
    assert!(PluginId::parse("user/a\"b").is_err());
}

// ── PluginId equality and hashing ─────────────────────────────────────────

#[test]
fn plugin_id_case_insensitive_equality() {
    assert_eq!(pid("foo/bar"), pid("FOO/BAR"));
    assert_eq!(pid("core:plum"), pid("core:PLUM"));
    assert_ne!(pid("foo/bar"), pid("foo/baz"));
    // Different variants are never equal.
    assert_ne!(pid("core:bar"), pid("foo/bar"));
}

#[test]
fn plugin_id_preserves_case_in_display() {
    assert_eq!(
        pid("SomeUser/CoolPlugin").to_string(),
        "SomeUser/CoolPlugin"
    );
    assert_eq!(
        pid("core:helix-surround").to_string(),
        "core:helix-surround"
    );
}

#[test]
fn plugin_id_equal_ids_have_equal_hashes() {
    use std::collections::hash_map::DefaultHasher;
    let hash_of = |id: &PluginId| {
        let mut h = DefaultHasher::new();
        id.hash(&mut h);
        h.finish()
    };
    assert_eq!(hash_of(&pid("Foo/Bar")), hash_of(&pid("foo/bar")));
    assert_eq!(hash_of(&pid("core:PLUM")), hash_of(&pid("core:plum")));
}

// ── PluginStack ──────────────────────────────────────────────────────────

#[test]
fn plugin_stack_empty_is_user() {
    let stack = PluginStack::default();
    assert_eq!(stack.current_owner(), Owner::User);
}

#[test]
fn plugin_stack_push_makes_plugin_owner() {
    let mut stack = PluginStack::default();
    let x = pid("user/x");
    stack.push(x.clone());
    assert_eq!(stack.current_owner(), Owner::Plugin(x));
}

#[test]
fn plugin_stack_pop_returns_to_user() {
    let mut stack = PluginStack::default();
    stack.push(pid("user/x"));
    stack.pop();
    assert_eq!(stack.current_owner(), Owner::User);
}

#[test]
fn plugin_stack_nested_plugins() {
    let mut stack = PluginStack::default();
    let x = pid("user/x");
    let y = pid("user/y");
    stack.push(x);
    stack.push(y.clone());
    assert_eq!(stack.current_owner(), Owner::Plugin(y));
    stack.pop();
    assert_eq!(stack.current_owner(), Owner::Plugin(pid("user/x")));
}

#[test]
fn plugin_stack_pop_on_empty_is_noop() {
    let mut stack = PluginStack::default();
    stack.pop(); // must not panic
    assert_eq!(stack.current_owner(), Owner::User);
}
