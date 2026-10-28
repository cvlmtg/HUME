use super::*;
use hume_core::ConfigOrigin;
use termina::event::{KeyCode, KeyEvent, KeyEventKind, Modifiers};

// ── bind_sequence / unbind_sequence / Keymap::bind / Keymap::unbind ──────

#[test]
fn bind_sequence_single_key() {
    let mut trie = KeyTrie::default();
    trie.bind_sequence(
        &[key!('z')],
        KeymapCommand {
            name: "my-cmd".into(),
            force_extend: false,
        },
    );
    assert!(
        matches!(trie.walk(&[key!('z')]), Some(WalkResult::Leaf(ref c)) if &*c.name == "my-cmd")
    );
}

#[test]
fn bind_sequence_multi_key() {
    let mut trie = KeyTrie::default();
    trie.bind_sequence(
        &[key!('g'), key!('g')],
        KeymapCommand {
            name: "goto-first-line".into(),
            force_extend: false,
        },
    );
    assert!(matches!(
        trie.walk(&[key!('g')]),
        Some(WalkResult::Interior)
    ));
    assert!(matches!(
        trie.walk(&[key!('g'), key!('g')]),
        Some(WalkResult::Leaf(ref c)) if &*c.name == "goto-first-line"
    ));
}

#[test]
fn bind_sequence_shadows_existing_leaf() {
    let mut trie = KeyTrie::default();
    // Bind `g` as a leaf first.
    trie.bind_sequence(
        &[key!('g')],
        KeymapCommand {
            name: "old-cmd".into(),
            force_extend: false,
        },
    );
    // Now bind `gg`; should convert `g` from Leaf to Node.
    trie.bind_sequence(
        &[key!('g'), key!('g')],
        KeymapCommand {
            name: "new-cmd".into(),
            force_extend: false,
        },
    );
    assert!(matches!(
        trie.walk(&[key!('g')]),
        Some(WalkResult::Interior)
    ));
    assert!(matches!(
        trie.walk(&[key!('g'), key!('g')]),
        Some(WalkResult::Leaf(ref c)) if &*c.name == "new-cmd"
    ));
}

#[test]
fn unbind_sequence_single_key_hides_it() {
    let mut trie = KeyTrie::default();
    trie.bind_sequence(
        &[key!('z')],
        KeymapCommand {
            name: "my-cmd".into(),
            force_extend: false,
        },
    );
    trie.unbind_sequence(&[key!('z')]);
    assert!(matches!(trie.walk(&[key!('z')]), Some(WalkResult::NoMatch)));
}

#[test]
fn unbind_sequence_multi_key_keeps_the_prefix() {
    let mut trie = KeyTrie::default();
    trie.bind_sequence(
        &[key!('g'), key!('g')],
        KeymapCommand {
            name: "goto-first-line".into(),
            force_extend: false,
        },
    );
    trie.unbind_sequence(&[key!('g'), key!('g')]);
    assert!(matches!(
        trie.walk(&[key!('g')]),
        Some(WalkResult::Interior)
    ));
    assert!(matches!(
        trie.walk(&[key!('g'), key!('g')]),
        Some(WalkResult::NoMatch)
    ));
}

#[test]
fn unbind_sequence_leaves_other_bindings() {
    let mut trie = KeyTrie::default();
    trie.bind_sequence(
        &[key!('z')],
        KeymapCommand {
            name: "my-cmd".into(),
            force_extend: false,
        },
    );
    trie.unbind_sequence(&[key!('q')]);
    assert!(
        matches!(trie.walk(&[key!('z')]), Some(WalkResult::Leaf(ref c)) if &*c.name == "my-cmd")
    );
}

#[test]
fn bind_in_normal_mode() {
    let mut km = Keymap::default();
    km.bind(
        BindMode::Normal,
        &[key!('z')],
        "my-cmd".into(),
        false,
        ConfigOrigin::User,
    );
    assert!(matches!(
        km.normal.walk(&[key!('z')]),
        WalkResult::Leaf(ref c) if &*c.name == "my-cmd"
    ));
    // Insert mode unchanged.
    assert!(matches!(km.insert.walk(&[key!('z')]), WalkResult::NoMatch));
}

#[test]
fn bind_in_command_mode() {
    let mut km = Keymap::default();
    km.bind(
        BindMode::Command,
        &[key!('z')],
        "my-cmd".into(),
        false,
        ConfigOrigin::User,
    );
    assert!(matches!(
        km.command.walk(&[key!('z')]),
        WalkResult::Leaf(ref c) if &*c.name == "my-cmd"
    ));
    assert!(!matches!(
        km.normal.walk(&[key!('z')]),
        WalkResult::Leaf(ref c) if &*c.name == "my-cmd"
    ));
}

#[test]
fn unbind_in_normal_mode() {
    let mut km = Keymap::default();
    km.bind(
        BindMode::Normal,
        &[key!('z')],
        "my-cmd".into(),
        false,
        ConfigOrigin::User,
    );
    km.unbind(BindMode::Normal, &[key!('z')], ConfigOrigin::User);
    assert!(matches!(km.normal.walk(&[key!('z')]), WalkResult::NoMatch));
}

// ── Origin layers ─────────────────────────────────────────────────────────

/// Bind `keys` to `name` in Normal mode from `by`, returning whether a
/// higher origin's binding hides it.
fn bind_from(km: &mut Keymap, keys: &[KeyEvent], name: &'static str, by: ConfigOrigin) -> bool {
    km.bind(BindMode::Normal, keys, name.into(), false, by)
}

/// The command `keys` resolve to in Normal mode.
fn resolved(km: &Keymap, keys: &[KeyEvent]) -> Option<String> {
    km.lookup_command(BindMode::Normal, keys)
        .map(|(name, _)| name)
}

#[test]
fn a_higher_origins_binding_wins_in_either_order() {
    for (higher, lower) in [
        (ConfigOrigin::User, ConfigOrigin::Plugin),
        (ConfigOrigin::Plugin, ConfigOrigin::Bundled),
    ] {
        for higher_first in [true, false] {
            let mut km = Keymap::empty();
            if higher_first {
                bind_from(&mut km, &[key!('K')], "higher", higher);
                bind_from(&mut km, &[key!('K')], "lower", lower);
            } else {
                bind_from(&mut km, &[key!('K')], "lower", lower);
                bind_from(&mut km, &[key!('K')], "higher", higher);
            }
            assert_eq!(
                resolved(&km, &[key!('K')]).as_deref(),
                Some("higher"),
                "{higher:?} over {lower:?}, higher_first={higher_first}"
            );
        }
    }
}

#[test]
fn a_binding_reports_whether_a_higher_origin_hides_it() {
    let mut km = Keymap::empty();
    assert!(!bind_from(
        &mut km,
        &[key!('K')],
        "user",
        ConfigOrigin::User
    ));
    assert!(bind_from(
        &mut km,
        &[key!('K')],
        "plugin",
        ConfigOrigin::Plugin
    ));
    assert!(!bind_from(
        &mut km,
        &[key!('J')],
        "plugin",
        ConfigOrigin::Plugin
    ));
}

#[test]
fn a_later_binding_from_one_origin_replaces_the_earlier() {
    let mut km = Keymap::empty();
    bind_from(&mut km, &[key!('K')], "first", ConfigOrigin::Plugin);
    bind_from(&mut km, &[key!('K')], "second", ConfigOrigin::Plugin);
    assert_eq!(resolved(&km, &[key!('K')]).as_deref(), Some("second"));
}

#[test]
fn a_users_unbind_hides_lower_bindings_made_before_and_after_it() {
    let mut km = Keymap::empty();
    bind_from(&mut km, &[key!('K')], "bundled", ConfigOrigin::Bundled);
    km.unbind(BindMode::Normal, &[key!('K')], ConfigOrigin::User);
    bind_from(&mut km, &[key!('K')], "plugin", ConfigOrigin::Plugin);
    assert_eq!(resolved(&km, &[key!('K')]), None);
}

#[test]
fn a_plugins_unbind_hides_a_bundled_binding_but_not_the_users() {
    let mut km = Keymap::empty();
    bind_from(
        &mut km,
        &[key!('m'), key!('w')],
        "bundled",
        ConfigOrigin::Bundled,
    );
    bind_from(&mut km, &[key!('K')], "user", ConfigOrigin::User);
    km.unbind(
        BindMode::Normal,
        &[key!('m'), key!('w')],
        ConfigOrigin::Plugin,
    );
    km.unbind(BindMode::Normal, &[key!('K')], ConfigOrigin::Plugin);
    assert_eq!(resolved(&km, &[key!('m'), key!('w')]), None);
    assert_eq!(resolved(&km, &[key!('K')]).as_deref(), Some("user"));
}

#[test]
fn bindings_under_one_prefix_from_two_origins_both_resolve() {
    let mut km = Keymap::empty();
    bind_from(
        &mut km,
        &[key!('g'), key!('d')],
        "bundled",
        ConfigOrigin::Bundled,
    );
    bind_from(&mut km, &[key!('g'), key!('x')], "user", ConfigOrigin::User);
    assert!(matches!(km.normal.walk(&[key!('g')]), WalkResult::Interior));
    assert_eq!(
        resolved(&km, &[key!('g'), key!('d')]).as_deref(),
        Some("bundled")
    );
    assert_eq!(
        resolved(&km, &[key!('g'), key!('x')]).as_deref(),
        Some("user")
    );
}

#[test]
fn binding_under_an_unbound_prefix_leaves_its_other_keys_unbound() {
    let mut km = Keymap::empty();
    bind_from(
        &mut km,
        &[key!('g'), key!('g')],
        "bundled",
        ConfigOrigin::Bundled,
    );
    km.unbind(BindMode::Normal, &[key!('g')], ConfigOrigin::User);
    bind_from(&mut km, &[key!('g'), key!('d')], "user", ConfigOrigin::User);
    assert_eq!(
        resolved(&km, &[key!('g'), key!('d')]).as_deref(),
        Some("user")
    );
    assert_eq!(resolved(&km, &[key!('g'), key!('g')]), None);
}

#[test]
fn a_higher_bare_key_hides_a_lower_subtree() {
    let mut km = Keymap::empty();
    bind_from(
        &mut km,
        &[key!('g'), key!('d')],
        "bundled",
        ConfigOrigin::Bundled,
    );
    bind_from(&mut km, &[key!('g')], "plugin", ConfigOrigin::Plugin);
    assert_eq!(resolved(&km, &[key!('g')]).as_deref(), Some("plugin"));
    assert_eq!(resolved(&km, &[key!('g'), key!('d')]), None);
}

// ── collect_command_names ─────────────────────────────────────────────────

#[test]
fn collect_command_names_includes_nested_leaves() {
    let mut trie = KeyTrie::default();
    trie.bind_sequence(
        &[key!('x')],
        KeymapCommand {
            name: "delete-char-forward".into(),
            force_extend: false,
        },
    );
    trie.bind_sequence(
        &[key!('g'), key!('g')],
        KeymapCommand {
            name: "goto-first-line".into(),
            force_extend: false,
        },
    );

    let mut names = Vec::new();
    trie.collect_command_names(&mut names);
    names.sort();

    assert_eq!(names, vec!["delete-char-forward", "goto-first-line"]);
}

#[test]
fn all_command_names_covers_every_mode() {
    let mut km = Keymap::default();
    // Bind sentinel names in each mode to verify the sweep is complete.
    km.bind(
        BindMode::Normal,
        &[key!('Q')],
        "normal-sentinel".into(),
        false,
        ConfigOrigin::User,
    );
    km.bind(
        BindMode::Insert,
        &[key!('Q')],
        "insert-sentinel".into(),
        false,
        ConfigOrigin::User,
    );
    km.bind(
        BindMode::Command,
        &[key!('Q')],
        "command-sentinel".into(),
        false,
        ConfigOrigin::User,
    );
    let names = km.all_command_names();
    assert!(
        names.contains(&"normal-sentinel".to_string()),
        "normal mode must be swept"
    );
    assert!(
        names.contains(&"insert-sentinel".to_string()),
        "insert mode must be swept"
    );
    assert!(
        names.contains(&"command-sentinel".to_string()),
        "command mode must be swept"
    );
}

// ── Trie resolution equivalence ───────────────────────────────────────────

#[test]
fn walk_resolves_uppercase_leaf_regardless_of_incoming_shift_bit() {
    let mut trie = KeyTrie::default();
    trie.bind_leaf(
        key!('I'),
        KeymapCommand {
            name: "insert-at-line-start".into(),
            force_extend: false,
        },
    );
    // A non-conformant kitty terminal delivers the uppercase codepoint
    // with SHIFT still set (see `handle_normal`'s doc comment); a clean
    // delivery omits it. Both must resolve to the same leaf.
    let non_conformant = KeyEvent::new(KeyCode::Char('I'), Modifiers::SHIFT);
    let clean = KeyEvent::new(KeyCode::Char('I'), Modifiers::NONE);
    assert!(matches!(
        trie.walk(&[non_conformant]),
        Some(WalkResult::Leaf(ref c)) if &*c.name == "insert-at-line-start"
    ));
    assert!(matches!(
        trie.walk(&[clean]),
        Some(WalkResult::Leaf(ref c)) if &*c.name == "insert-at-line-start"
    ));
}

#[test]
fn walk_resolves_repeat_kind_key_like_press() {
    // A kitty terminal with REPORT_EVENT_TYPES sends autorepeat as
    // `KeyEventKind::Repeat`, not `Press`. A held key must keep matching
    // its binding.
    let mut trie = KeyTrie::default();
    trie.bind_leaf(
        key!('j'),
        KeymapCommand {
            name: "move-down".into(),
            force_extend: false,
        },
    );
    let mut repeated = KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE);
    repeated.kind = KeyEventKind::Repeat;
    assert!(matches!(
        trie.walk(&[repeated]),
        Some(WalkResult::Leaf(ref c)) if &*c.name == "move-down"
    ));
}

#[test]
fn bind_shift_lowercase_and_uppercase_are_the_same_binding() {
    // Steel's `bind-key!` "shift-a" / "S-a" spelling parses to
    // Char('a')+SHIFT (see hume-scripting's `keys.rs`); a plain "A" bind
    // string and live uppercase keypresses both resolve to Char('A')+SHIFT
    // once canonicalized. All three must be the same trie entry.
    let mut km = Keymap::default();
    km.bind(
        BindMode::Normal,
        &[KeyEvent::new(KeyCode::Char('a'), Modifiers::SHIFT)],
        "my-cmd".into(),
        false,
        ConfigOrigin::User,
    );
    assert!(matches!(
        km.normal.walk(&[key!('A')]),
        WalkResult::Leaf(ref c) if &*c.name == "my-cmd"
    ));
}
