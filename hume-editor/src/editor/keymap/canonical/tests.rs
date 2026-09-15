use super::*;
use termina::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, Modifiers};

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

// ── CanonicalKey Hash/Eq ─────────────────────────────────────────────────

#[test]
fn equal_canonical_keys_hash_equally() {
    // Two raw KeyEvents that canonicalization collapses to the same binding
    // identity must also collapse to the same CanonicalKey, and produce
    // equal hashes — Eq and Hash both read `encode`, so this is exactly the
    // contract a derived Eq used to carry only because `canonical` happens
    // to pin the fields it would otherwise also compare.
    let a = CanonicalKey::from(KeyEvent::new(KeyCode::Char('G'), Modifiers::NONE));
    let b = CanonicalKey::from(KeyEvent::new(KeyCode::Char('G'), Modifiers::SHIFT));
    assert_eq!(a, b);

    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher_a = DefaultHasher::new();
    a.hash(&mut hasher_a);
    let mut hasher_b = DefaultHasher::new();
    b.hash(&mut hasher_b);
    assert_eq!(hasher_a.finish(), hasher_b.finish());
}
