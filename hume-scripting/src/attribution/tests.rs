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

// ── AttributionStack ─────────────────────────────────────────────────────

fn top(stack: &AttributionStack) -> Option<Owner> {
    stack.top_owner().map(|owner| Owner::clone(owner))
}

#[test]
fn an_empty_stack_names_no_owner() {
    assert_eq!(top(&AttributionStack::default()), None);
}

#[test]
fn a_body_runs_as_its_plugin() {
    let mut stack = AttributionStack::default();
    let x = pid("user/x");
    stack.push_body(EntryId::main(x.clone()));
    assert_eq!(top(&stack), Some(Owner::Plugin(EntryId::main(x))));
    assert!(stack.in_body());
}

#[test]
fn popping_a_body_returns_its_entry() {
    let mut stack = AttributionStack::default();
    stack.push_body(EntryId::main(pid("user/x")));
    assert_eq!(stack.pop_body(), Some(EntryId::main(pid("user/x"))));
    assert_eq!(top(&stack), None);
    assert!(!stack.in_body());
}

#[test]
fn nested_bodies_run_innermost_first() {
    let mut stack = AttributionStack::default();
    stack.push_body(EntryId::main(pid("user/x")));
    stack.push_body(EntryId::main(pid("user/y")));
    assert_eq!(
        top(&stack),
        Some(Owner::Plugin(EntryId::main(pid("user/y"))))
    );
    assert_eq!(stack.body_depth(), 2);
    stack.pop_body();
    assert_eq!(
        top(&stack),
        Some(Owner::Plugin(EntryId::main(pid("user/x"))))
    );
}

#[test]
fn pop_body_on_empty_returns_none() {
    assert_eq!(AttributionStack::default().pop_body(), None);
}

/// A `call!` frame runs as the command's definer, and is not a body: it
/// neither opens a plugin body's gates nor counts toward the body depth.
#[test]
fn a_call_frame_runs_as_its_owner_and_is_not_a_body() {
    let mut stack = AttributionStack::default();
    stack.push_body(EntryId::main(pid("user/x")));
    let depth = stack.enter_call(std::rc::Rc::new(Owner::User));
    assert_eq!(top(&stack), Some(Owner::User));
    assert_eq!(stack.body_depth(), 1);
    assert_eq!(stack.pop_body(), None, "the innermost frame is the call");
    assert_eq!(
        stack.innermost_body().map(|entry| &**entry),
        Some(&EntryId::main(pid("user/x")))
    );
    assert!(stack.drain_above(depth).is_empty());
    assert_eq!(
        top(&stack),
        Some(Owner::Plugin(EntryId::main(pid("user/x"))))
    );
}

/// A session's end drops its call frames and hands back its bodies,
/// innermost first.
#[test]
fn drain_above_returns_the_bodies_innermost_first() {
    let mut stack = AttributionStack::default();
    stack.push_body(EntryId::main(pid("user/x")));
    stack.enter_call(std::rc::Rc::new(Owner::User));
    stack.push_body(EntryId::main(pid("user/y")));
    assert_eq!(
        stack.drain_above(0),
        vec![EntryId::main(pid("user/y")), EntryId::main(pid("user/x"))]
    );
    assert_eq!(stack.len(), 0);
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
fn entry_file_equality_is_case_sensitive() {
    let a = EntryFile::parse("Install.scm").unwrap();
    let b = EntryFile::parse("install.scm").unwrap();
    assert_ne!(a, b);
    let mut set = std::collections::HashSet::new();
    set.insert(a);
    assert!(!set.contains(&b));
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

/// HUME's own runtime files are HUME's: a command one defines is owned by
/// `hume`, while one `init.scm` defines is the user's.
#[test]
fn a_bundled_runtime_file_attributes_to_hume() {
    use crate::{ScriptingHost, null_host::CommandStoreHost};
    use hume_platform::dirs::Dirs;
    use steel::rvals::SteelVal;
    let dir = tempfile::tempdir().unwrap();
    let bundled = dir.path().join("bundled.scm");
    std::fs::write(
        &bundled,
        r#"(define-command! "bundled-cmd" "" (lambda () #t))"#,
    )
    .unwrap();
    let init = dir.path().join("init.scm");
    std::fs::write(&init, r#"(define-command! "init-cmd" "" (lambda () #t))"#).unwrap();

    let mut host = ScriptingHost::new(&Dirs::none());
    let mut editor_host = CommandStoreHost::default();
    host.eval_runtime(&bundled, 10_000, &mut editor_host)
        .expect("the bundled file evaluates");
    host.eval_init(&init, 10_000, &mut editor_host)
        .expect("init.scm evaluates");

    let owners = host.eval_source_for_value(
        r#"(list (command-plugin "bundled-cmd") (command-plugin "init-cmd"))"#,
        &mut editor_host,
    );
    assert_eq!(
        owners,
        Ok(SteelVal::ListV(
            vec![
                SteelVal::StringV("hume".into()),
                SteelVal::StringV("user".into())
            ]
            .into()
        ))
    );
}
