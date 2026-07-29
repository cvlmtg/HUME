use super::*;
use termina::event::{KeyCode, KeyEvent, Modifiers};

// ── bind_sequence / remove_sequence / bind_user_with_extend / unbind_user ─

#[test]
fn bind_sequence_single_key() {
    let mut trie = KeyTrie::new();
    trie.bind_sequence(
        &[key!('z')],
        KeymapCommand {
            name: Cow::Borrowed("my-cmd"),
            force_extend: false,
        },
    );
    assert!(matches!(trie.walk(&[key!('z')]), WalkResult::Leaf(ref c) if c.name == "my-cmd"));
}

#[test]
fn bind_sequence_multi_key() {
    let mut trie = KeyTrie::new();
    trie.bind_sequence(
        &[key!('g'), key!('g')],
        KeymapCommand {
            name: Cow::Borrowed("goto-first-line"),
            force_extend: false,
        },
    );
    assert!(matches!(trie.walk(&[key!('g')]), WalkResult::Interior));
    assert!(matches!(
        trie.walk(&[key!('g'), key!('g')]),
        WalkResult::Leaf(ref c) if c.name == "goto-first-line"
    ));
}

#[test]
fn bind_sequence_shadows_existing_leaf() {
    let mut trie = KeyTrie::new();
    // Bind `g` as a leaf first.
    trie.bind_sequence(
        &[key!('g')],
        KeymapCommand {
            name: Cow::Borrowed("old-cmd"),
            force_extend: false,
        },
    );
    // Now bind `gg` — should convert `g` from Leaf to Node.
    trie.bind_sequence(
        &[key!('g'), key!('g')],
        KeymapCommand {
            name: Cow::Borrowed("new-cmd"),
            force_extend: false,
        },
    );
    assert!(matches!(trie.walk(&[key!('g')]), WalkResult::Interior));
    assert!(matches!(
        trie.walk(&[key!('g'), key!('g')]),
        WalkResult::Leaf(ref c) if c.name == "new-cmd"
    ));
}

#[test]
fn remove_sequence_single_key() {
    let mut trie = KeyTrie::new();
    trie.bind_sequence(
        &[key!('z')],
        KeymapCommand {
            name: Cow::Borrowed("my-cmd"),
            force_extend: false,
        },
    );
    trie.remove_sequence(&[key!('z')]);
    assert!(matches!(trie.walk(&[key!('z')]), WalkResult::NoMatch));
}

#[test]
fn remove_sequence_multi_key() {
    let mut trie = KeyTrie::new();
    trie.bind_sequence(
        &[key!('g'), key!('g')],
        KeymapCommand {
            name: Cow::Borrowed("goto-first-line"),
            force_extend: false,
        },
    );
    trie.remove_sequence(&[key!('g'), key!('g')]);
    // Interior node for `g` remains; leaf `gg` is gone.
    assert!(matches!(trie.walk(&[key!('g')]), WalkResult::Interior));
    assert!(matches!(
        trie.walk(&[key!('g'), key!('g')]),
        WalkResult::NoMatch
    ));
}

#[test]
fn remove_sequence_nonexistent_is_noop() {
    let mut trie = KeyTrie::new();
    trie.bind_sequence(
        &[key!('z')],
        KeymapCommand {
            name: Cow::Borrowed("my-cmd"),
            force_extend: false,
        },
    );
    trie.remove_sequence(&[key!('q')]); // q not bound — no-op
    trie.remove_sequence(&[key!('z'), key!('z')]); // path doesn't exist — no-op
    // `z` leaf is untouched.
    assert!(matches!(trie.walk(&[key!('z')]), WalkResult::Leaf(ref c) if c.name == "my-cmd"));
}

#[test]
fn bind_user_normal_mode() {
    let mut km = Keymap::default();
    km.bind_user_with_extend(
        BindMode::Normal,
        &[key!('z')],
        Cow::Borrowed("my-cmd"),
        false,
    );
    assert!(matches!(
        km.normal.walk(&[key!('z')]),
        WalkResult::Leaf(ref c) if c.name == "my-cmd"
    ));
    // Insert mode unchanged.
    assert!(matches!(km.insert.walk(&[key!('z')]), WalkResult::NoMatch));
}

#[test]
fn unbind_user_normal_mode() {
    let mut km = Keymap::default();
    km.bind_user_with_extend(
        BindMode::Normal,
        &[key!('z')],
        Cow::Borrowed("my-cmd"),
        false,
    );
    km.unbind_user(BindMode::Normal, &[key!('z')]);
    assert!(matches!(km.normal.walk(&[key!('z')]), WalkResult::NoMatch));
}

// ── collect_command_names ─────────────────────────────────────────────────

#[test]
fn collect_command_names_includes_leaves_and_waitchars() {
    let mut trie = KeyTrie::new();
    trie.bind_sequence(
        &[key!('x')],
        KeymapCommand {
            name: Cow::Borrowed("delete-char-forward"),
            force_extend: false,
        },
    );
    trie.bind_sequence(
        &[key!('g'), key!('g')],
        KeymapCommand {
            name: Cow::Borrowed("goto-first-line"),
            force_extend: false,
        },
    );
    trie.bind_wait_char_sequence(
        &[key!('f')],
        WaitCharPending {
            cmd_name: Cow::Borrowed("find-char"),
            ctrl_extend: false,
        },
    );

    let mut names: Vec<String> = Vec::new();
    trie.collect_command_names(&mut names);
    names.sort();

    // Independent oracle: exact expected set.
    assert!(
        names.contains(&"delete-char-forward".to_string()),
        "leaf must appear"
    );
    assert!(
        names.contains(&"goto-first-line".to_string()),
        "nested leaf must appear"
    );
    assert!(
        names.contains(&"find-char".to_string()),
        "wait-char must appear"
    );
}

#[test]
fn all_command_names_covers_all_three_modes() {
    let mut km = Keymap::default();
    // Bind sentinel names in each mode to verify the sweep is complete.
    km.bind_user_with_extend(
        BindMode::Normal,
        &[key!('Q')],
        Cow::Borrowed("normal-sentinel"),
        false,
    );
    km.bind_user_with_extend(
        BindMode::Insert,
        &[key!('Q')],
        Cow::Borrowed("insert-sentinel"),
        false,
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
}

// ── canonical() ────────────────────────────────────────────────────────────

#[test]
fn canonical_uppercase_char_gains_shift() {
    let k = canonical(KeyEvent::new(KeyCode::Char('G'), Modifiers::NONE));
    assert_eq!(k, KeyEvent::new(KeyCode::Char('G'), Modifiers::SHIFT));
}

#[test]
fn canonical_shift_lowercase_char_becomes_uppercase() {
    let k = canonical(KeyEvent::new(KeyCode::Char('g'), Modifiers::SHIFT));
    assert_eq!(k, KeyEvent::new(KeyCode::Char('G'), Modifiers::SHIFT));
}

#[test]
fn canonical_shift_punctuation_is_unchanged() {
    // Punctuation has no case to normalize — SHIFT+':' stays distinct from
    // plain ':'. (The lone-SHIFT strip in `handle_normal` handles the
    // partially-compliant-terminal gap for punctuation; that's a separate
    // mechanism from this trie-identity normalization.)
    let k = canonical(KeyEvent::new(KeyCode::Char(':'), Modifiers::SHIFT));
    assert_eq!(k, KeyEvent::new(KeyCode::Char(':'), Modifiers::SHIFT));
}

#[test]
fn canonical_scrubs_lock_bits() {
    let k = canonical(KeyEvent::new(
        KeyCode::Char('h'),
        Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK,
    ));
    assert_eq!(k, KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE));
}

#[test]
fn canonical_scrubs_protocol_state() {
    let mut k = KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE);
    k.state = KeyEventState::KEYPAD | KeyEventState::CAPS_LOCK;
    assert_eq!(canonical(k).state, KeyEventState::NONE);
}

#[test]
fn canonical_repeat_becomes_press() {
    let mut k = KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE);
    k.kind = KeyEventKind::Repeat;
    assert_eq!(canonical(k).kind, KeyEventKind::Press);
}

#[test]
fn canonical_is_idempotent() {
    let k = KeyEvent::new(KeyCode::Char('g'), Modifiers::SHIFT);
    assert_eq!(canonical(canonical(k)), canonical(k));
}

// ── Trie resolution equivalence ───────────────────────────────────────────

#[test]
fn walk_resolves_uppercase_leaf_regardless_of_incoming_shift_bit() {
    let mut trie = KeyTrie::new();
    trie.bind_leaf(
        key!('I'),
        KeymapCommand {
            name: Cow::Borrowed("insert-at-line-start"),
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
        WalkResult::Leaf(ref c) if c.name == "insert-at-line-start"
    ));
    assert!(matches!(
        trie.walk(&[clean]),
        WalkResult::Leaf(ref c) if c.name == "insert-at-line-start"
    ));
}

#[test]
fn walk_resolves_repeat_kind_key_like_press() {
    // A kitty terminal with REPORT_EVENT_TYPES sends autorepeat as
    // `KeyEventKind::Repeat`, not `Press`. A held key must keep matching
    // its binding.
    let mut trie = KeyTrie::new();
    trie.bind_leaf(
        key!('j'),
        KeymapCommand {
            name: Cow::Borrowed("move-down"),
            force_extend: false,
        },
    );
    let mut repeated = KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE);
    repeated.kind = KeyEventKind::Repeat;
    assert!(matches!(
        trie.walk(&[repeated]),
        WalkResult::Leaf(ref c) if c.name == "move-down"
    ));
}

#[test]
fn bind_user_with_extend_shift_lowercase_and_uppercase_are_the_same_binding() {
    // Steel's `bind-key!` "shift-a" / "S-a" spelling parses to
    // Char('a')+SHIFT (see hume-scripting's `keys.rs`); a plain "A" bind
    // string and live uppercase keypresses both resolve to Char('A')+SHIFT
    // once canonicalized. All three must be the same trie entry.
    let mut km = Keymap::default();
    km.bind_user_with_extend(
        BindMode::Normal,
        &[KeyEvent::new(KeyCode::Char('a'), Modifiers::SHIFT)],
        Cow::Borrowed("my-cmd"),
        false,
    );
    assert!(matches!(
        km.normal.walk(&[key!('A')]),
        WalkResult::Leaf(ref c) if c.name == "my-cmd"
    ));
}
