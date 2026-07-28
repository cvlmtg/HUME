use super::super::{Keymap, WalkResult};
use super::{default_insert_keymap, default_normal_keymap};
use termina::event::{KeyCode, KeyEvent, Modifiers};

#[test]
fn no_match() {
    let trie = default_normal_keymap();
    // `~` is not bound.
    assert!(matches!(trie.walk(&[key!('~')]), WalkResult::NoMatch));
    // `m` + `z` is not a valid text object sequence.
    assert!(matches!(
        trie.walk(&[key!('m'), key!('z')]),
        WalkResult::NoMatch
    ));
    // Too many keys for a leaf binding.
    assert!(matches!(
        trie.walk(&[key!('h'), key!('j')]),
        WalkResult::NoMatch
    ));
}

// ── Insert keymap ─────────────────────────────────────────────────────────

#[test]
fn insert_char_is_no_match() {
    // Regular characters are NOT in the insert trie — they fall through
    // to the char-insertion handler in the dispatcher.
    let trie = default_insert_keymap();
    assert!(matches!(trie.walk(&[key!('a')]), WalkResult::NoMatch));
    assert!(matches!(trie.walk(&[key!('z')]), WalkResult::NoMatch));
}

#[test]
fn ctrl_bindings_in_normal_keymap() {
    let trie = default_normal_keymap();
    // Ctrl+c is intentionally unbound in normal mode — force-quit must be
    // invoked via :quit or :q to avoid accidental data loss.
    assert!(
        matches!(trie.walk(&[key!(Ctrl + 'c')]), WalkResult::NoMatch),
        "Ctrl+c must be unbound in normal mode"
    );
    assert!(
        matches!(trie.walk(&[key!(Ctrl + 'r')]), WalkResult::Leaf(ref cmd) if cmd.name == "redo"),
        "Ctrl+r should map to redo"
    );
    assert!(
        matches!(trie.walk(&[key!(Ctrl + 'x')]), WalkResult::Leaf(ref cmd) if cmd.name == "select-line"),
        "Ctrl+x should map to select-line"
    );
    // Ctrl+e flips anchor↔head and works on legacy terminals (0x05 control byte).
    assert!(
        matches!(trie.walk(&[key!(Ctrl + 'e')]), WalkResult::Leaf(ref cmd) if cmd.name == "flip-selections"),
        "Ctrl+e should map to flip-selections"
    );
    // Ctrl+w is deliberately unbound (kitty one-shot extend via strip-CONTROL).
    assert!(
        matches!(trie.walk(&[key!(Ctrl + 'w')]), WalkResult::NoMatch),
        "Ctrl+w must be unbound — pane prefix is Ctrl+p"
    );
    // Ctrl+p is the pane prefix (Interior node).
    assert!(
        matches!(trie.walk(&[key!(Ctrl + 'p')]), WalkResult::Interior),
        "Ctrl+p must be the pane prefix Interior node"
    );
    // Ctrl+/ → search-selection (kitty-only).
    assert!(
        matches!(trie.walk(&[key!(Ctrl + '/')]), WalkResult::Leaf(ref cmd) if cmd.name == "search-selection"),
        "Ctrl+/ should map to search-selection"
    );
    // Legacy terminals encode Ctrl+/ as Ctrl+'7' (control byte 0x1F) — must
    // stay unbound rather than accidentally aliasing to search-selection.
    assert!(
        matches!(trie.walk(&[key!(Ctrl + '7')]), WalkResult::NoMatch),
        "Ctrl+7 must be unbound — Ctrl+/ is kitty-only, no legacy alias"
    );
}

/// Explicit Ctrl+x/X must carry force_extend=true; scroll and jump bindings must not.
#[test]
fn force_extend_flags_are_correct() {
    let trie = default_normal_keymap();

    let WalkResult::Leaf(cx) = trie.walk(&[key!(Ctrl + 'x')]) else {
        panic!()
    };
    assert!(
        cx.force_extend,
        "Ctrl+x (select-line) must have force_extend=true"
    );

    let WalkResult::Leaf(cx_upper) = trie.walk(&[key!(Ctrl + 'X')]) else {
        panic!()
    };
    assert!(
        cx_upper.force_extend,
        "Ctrl+X (select-line-backward) must have force_extend=true"
    );

    let WalkResult::Leaf(cd) = trie.walk(&[key!(Ctrl + 'd')]) else {
        panic!()
    };
    assert!(
        !cd.force_extend,
        "Ctrl+d (half-page-down) must have force_extend=false"
    );

    let WalkResult::Leaf(cu) = trie.walk(&[key!(Ctrl + 'u')]) else {
        panic!()
    };
    assert!(
        !cu.force_extend,
        "Ctrl+u (half-page-up) must have force_extend=false"
    );

    let WalkResult::Leaf(co) = trie.walk(&[key!(Ctrl + 'o')]) else {
        panic!()
    };
    assert!(
        !co.force_extend,
        "Ctrl+o (jump-backward) must have force_extend=false"
    );
}

#[test]
fn essential_keys_are_bound() {
    let trie = default_normal_keymap();
    // Spot-check a set of keys that would be ambiguous if duplicated.
    let must_be_bound = [
        key!('h'),
        key!('j'),
        key!('k'),
        key!('l'),
        key!('w'),
        key!('W'),
        key!('b'),
        key!('B'),
        key!('d'),
        key!('c'),
        key!('y'),
        key!('u'),
        key!('i'),
        key!('a'),
        key!('o'),
        key!('O'),
        key!('x'),
        key!('X'),
        key!('p'),
        key!('P'),
        key!('f'),
        key!('t'),
        key!('F'),
        key!('T'),
        key!('r'),
        key!('e'),
        key!(';'),
        key!(','),
        key!(Ctrl + 'r'),
        key!(Ctrl + 'x'),
        key!(Ctrl + 'o'),
        key!(Ctrl + 'i'),
        key!(Tab),
        key!(Ctrl + 'p'), // pane prefix (Interior)
        key!(Ctrl + 'e'), // flip-selections
    ];
    for k in must_be_bound {
        assert!(
            !matches!(trie.walk(&[k]), WalkResult::NoMatch),
            "key {:?} unexpectedly unbound in normal keymap",
            k
        );
    }
}

// ── Kitty-only default binds ──────────────────────────────────────────────

#[test]
fn default_keymap_omits_kitty_only_binds() {
    let km = Keymap::default();
    // Ctrl+; and Ctrl+, must NOT be present in the legacy-accurate default
    // trie — they are installed by apply_kitty_defaults only when the kitty
    // probe succeeds.
    assert!(
        matches!(km.normal.walk(&[key!(Ctrl + ';')]), WalkResult::NoMatch),
        "Ctrl+; must be unbound in default keymap (legacy mode)"
    );
    assert!(
        matches!(km.normal.walk(&[key!(Ctrl + ',')]), WalkResult::NoMatch),
        "Ctrl+, must be unbound in default keymap (legacy mode)"
    );
    // The plain-key counterparts remain bound regardless of kitty mode.
    assert!(
        matches!(km.normal.walk(&[key!(';')]), WalkResult::Leaf(ref c) if c.name == "collapse-and-exit-extend")
    );
    assert!(
        matches!(km.normal.walk(&[key!(',')]), WalkResult::Leaf(ref c) if c.name == "keep-primary-selection")
    );
}

#[test]
fn apply_kitty_defaults_binds_kitty_only_keys() {
    let mut km = Keymap::default();
    km.apply_kitty_defaults();
    let WalkResult::Leaf(c) = km.normal.walk(&[key!(Ctrl + ';')]) else {
        panic!("Ctrl+; should bind to collapse-to-anchor-and-exit-extend");
    };
    assert_eq!(c.name, "collapse-to-anchor-and-exit-extend");
    let WalkResult::Leaf(c) = km.normal.walk(&[key!(Ctrl + ',')]) else {
        panic!("Ctrl+, should bind to remove-primary-selection");
    };
    assert_eq!(c.name, "remove-primary-selection");
    let WalkResult::Leaf(c) = km.normal.walk(&[key!(Tab)]) else {
        panic!("Tab should bind to pane-focus-next under kitty defaults");
    };
    assert_eq!(c.name, "pane-focus-next");
}
