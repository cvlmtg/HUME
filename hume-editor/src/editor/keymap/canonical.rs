//! Canonical binding identity for key events.
//!
//! `CanonicalKey`'s field is private to this file (not just to `keymap`), so
//! `From<KeyEvent>` — which always canonicalizes — is the only way anywhere
//! in the crate to construct one. That makes "every `CanonicalKey` is
//! canonical" a compiler-enforced invariant rather than a convention each new
//! consumer (the trie, `PickerSession::actions`, …) has to uphold by hand.

use termina::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, Modifiers};

/// `KeyEvent`'s `PartialEq`/`Hash` impls perform no case normalization, so
/// binding lookups normalize uppercase char ⇔ `SHIFT` explicitly at this
/// boundary. Also scrubs fields that never participate in binding identity:
/// `kind` (a kitty autorepeat is a `Repeat` event, not `Press` — held keys
/// must keep matching the same binding under `REPORT_EVENT_TYPES`), protocol
/// `state`, and the Caps/Num Lock modifier bits.
fn canonical(mut key: KeyEvent) -> KeyEvent {
    key.kind = KeyEventKind::Press;
    key.state = KeyEventState::NONE;
    key.modifiers -= Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK;
    if let KeyCode::Char(c) = key.code {
        if c.is_ascii_uppercase() {
            key.modifiers |= Modifiers::SHIFT;
        } else if key.modifiers.contains(Modifiers::SHIFT) {
            // No-op for punctuation and other non-alphabetic chars: shifted
            // punctuation (e.g. `:`) stays distinct from its unshifted form,
            // matching what terminals actually deliver.
            key.code = KeyCode::Char(c.to_ascii_uppercase());
        }
    }
    key
}

/// Tags each [`KeyCode`] variant with a small integer plus payload so
/// [`encode`] can pack it into a `u64`. `Media`/`Modifier` are fieldless enums,
/// so casting to `u32` is a plain discriminant read.
fn encode_key_code(code: KeyCode) -> (u8, u32) {
    match code {
        KeyCode::Char(c) => (0, c as u32),
        KeyCode::Function(n) => (1, n as u32),
        KeyCode::Media(m) => (2, m as u32),
        KeyCode::Modifier(m) => (3, m as u32),
        KeyCode::Enter => (4, 0),
        KeyCode::Backspace => (5, 0),
        KeyCode::Tab => (6, 0),
        KeyCode::Escape => (7, 0),
        KeyCode::Left => (8, 0),
        KeyCode::Right => (9, 0),
        KeyCode::Up => (10, 0),
        KeyCode::Down => (11, 0),
        KeyCode::Home => (12, 0),
        KeyCode::End => (13, 0),
        KeyCode::BackTab => (14, 0),
        KeyCode::PageUp => (15, 0),
        KeyCode::PageDown => (16, 0),
        KeyCode::Insert => (17, 0),
        KeyCode::Delete => (18, 0),
        KeyCode::KeypadBegin => (19, 0),
        KeyCode::CapsLock => (20, 0),
        KeyCode::ScrollLock => (21, 0),
        KeyCode::NumLock => (22, 0),
        KeyCode::PrintScreen => (23, 0),
        KeyCode::Pause => (24, 0),
        KeyCode::Menu => (25, 0),
        KeyCode::Null => (26, 0),
    }
}

/// Injective encoding of a canonical key event's `(code, modifiers)` pair
/// into a `u64`. `kind`/`state` are excluded because [`canonical`] already
/// collapses them to fixed values. Injectivity rests on two facts checked
/// below rather than asserted here: [`encode_key_code`]'s 27 tags are
/// pairwise distinct (checked by the
/// `encoding_is_injective_across_key_code_variants` test, since nothing about
/// a hand-numbered match arm list is compiler-checked), and `modifiers.bits()`
/// fits in the low 8 bits the tag/payload shift leaves for it (checked by the
/// `const` assertion below) — termina 0.4.0's `Modifiers` is a `u8` bitflags
/// using every bit, so this is exact, not a margin.
fn encode(key: &KeyEvent) -> u64 {
    let (tag, payload) = encode_key_code(key.code);
    ((tag as u64) << 40) | ((payload as u64) << 8) | key.modifiers.bits() as u64
}

// A version bump widening `Modifiers` for a new modifier bit would spill
// `bits()` past the 8 bits `encode` reserves for it into the payload shift,
// silently making `Ctrl-Alt-<char>` (say) collide with some other key —
// caught here at compile time instead of via a runtime collision.
const _: () = assert!(Modifiers::all().bits() as u64 <= 0xFF);

/// [`encode`]'s output, wrapped so `Eq`/`Hash` agreement is a derive instead
/// of two hand-written impls that agree only by construction. Case-normalized
/// via [`canonical`] on the only path into one (`From<KeyEvent>`) — termina's
/// `KeyEvent` has no case-normalized equality of its own, which binding
/// lookup needs, so this type is the sole place a raw `KeyEvent` becomes a
/// binding identity. The wrapped `u64` is never read back — every consumer
/// (`KeyTrie`'s map, `PickerSession::actions`) only ever compares or hashes
/// one — so nothing is lost by not keeping the original `KeyEvent` around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::editor) struct CanonicalKey(u64);

impl From<KeyEvent> for CanonicalKey {
    fn from(key: KeyEvent) -> Self {
        Self(encode(&canonical(key)))
    }
}

#[cfg(test)]
mod tests;
