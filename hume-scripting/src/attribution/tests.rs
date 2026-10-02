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
    stack.push(EntryId::main(x.clone()));
    assert_eq!(stack.current_owner(), Owner::Plugin(EntryId::main(x)));
}

#[test]
fn plugin_stack_pop_returns_to_user() {
    let mut stack = PluginStack::default();
    stack.push(EntryId::main(pid("user/x")));
    stack.pop();
    assert_eq!(stack.current_owner(), Owner::User);
}

#[test]
fn plugin_stack_nested_plugins() {
    let mut stack = PluginStack::default();
    let x = pid("user/x");
    let y = pid("user/y");
    stack.push(EntryId::main(x));
    stack.push(EntryId::main(y.clone()));
    assert_eq!(stack.current_owner(), Owner::Plugin(EntryId::main(y)));
    stack.pop();
    assert_eq!(
        stack.current_owner(),
        Owner::Plugin(EntryId::main(pid("user/x")))
    );
}

#[test]
fn plugin_stack_pop_on_empty_is_noop() {
    let mut stack = PluginStack::default();
    stack.pop(); // must not panic
    assert_eq!(stack.current_owner(), Owner::User);
}

// ── EntryFile / EntryId ───────────────────────────────────────────────────

#[test]
fn entry_file_main_is_plugin_scm() {
    assert_eq!(EntryFile::main().as_str(), "plugin.scm");
}

#[test]
fn entry_file_accepts_a_scm_segment() {
    assert_eq!(
        EntryFile::parse("commands.scm").unwrap().as_str(),
        "commands.scm"
    );
}

#[test]
fn entry_file_rejects_non_segments_and_non_scm() {
    for bad in [
        "", ".scm", "../x.scm", "a/b.scm", "a\\b.scm", "x.txt", "x", "x\".scm",
    ] {
        assert!(EntryFile::parse(bad).is_err(), "{bad:?} must be rejected");
    }
}

#[test]
fn entry_file_equality_ignores_ascii_case() {
    let a = EntryFile::parse("Install.scm").unwrap();
    let b = EntryFile::parse("install.scm").unwrap();
    assert_eq!(a, b);
    let mut set = std::collections::HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}

#[test]
fn entry_id_distinguishes_entries_of_one_plugin() {
    let main = EntryId::main(pid("core:p"));
    let other = EntryId::new(pid("core:p"), EntryFile::parse("x.scm").unwrap());
    assert_ne!(main, other);
    assert_eq!(main.plugin, other.plugin);
}

#[test]
fn entry_id_display_names_the_file_only_for_secondary_entries() {
    let main = EntryId::main(pid("core:p"));
    let other = EntryId::new(pid("core:p"), EntryFile::parse("x.scm").unwrap());
    assert_eq!(main.to_string(), "core:p");
    assert_eq!(other.to_string(), "core:p (x.scm)");
}

// ── PluginId::Local ───────────────────────────────────────────────────────

#[test]
fn parse_local_accepts_files_beside_init() {
    for name in ["./a.scm", "./sub/a.scm", "./Sub/My-File.scm"] {
        let id = PluginId::parse(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(id.is_local(), "{name}");
        assert_eq!(
            id.to_string(),
            name,
            "Display must round-trip through parse"
        );
    }
}

#[test]
fn parse_local_rejects_unsafe_or_non_scm_paths() {
    for name in [
        "a.scm",
        "/a.scm",
        "./../a.scm",
        "./sub/../a.scm",
        "././a.scm",
        ".//a.scm",
        "./.scm",
        "./a",
        "./a.txt",
        ".\\a.scm",
        "./a\"b.scm",
        "../a.scm",
    ] {
        assert!(PluginId::parse(name).is_err(), "{name} must be rejected");
    }
}

#[test]
fn local_ids_compare_case_insensitively_and_never_equal_installed_ids() {
    assert_eq!(pid("./Foo.scm"), pid("./foo.scm"));
    assert_ne!(pid("./a.scm"), pid("core:a"));
    let hash = |id: &PluginId| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut h);
        h.finish()
    };
    assert_eq!(hash(&pid("./Foo.scm")), hash(&pid("./foo.scm")));
}

#[test]
fn parse_installed_rejects_local_ids() {
    assert!(PluginId::parse_installed("./a.scm").is_err());
    assert!(PluginId::parse_installed("core:a").is_ok());
    assert!(PluginId::parse_installed("alice/bar").is_ok());
}
