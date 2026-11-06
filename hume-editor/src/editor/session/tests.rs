use pretty_assertions::assert_eq;
use termina::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, MediaKeyCode, ModifierKeyCode, Modifiers,
};

use super::keys::SavedKey;

fn round_trip(key: KeyEvent) -> KeyEvent {
    let bytes = postcard::to_stdvec(&SavedKey::from(key)).expect("a key encodes");
    let saved: SavedKey = postcard::from_bytes(&bytes).expect("a key decodes");
    saved.into()
}

#[test]
fn every_key_code_survives_saving() {
    use MediaKeyCode as Media;
    use ModifierKeyCode as Modifier;
    let codes = [
        KeyCode::Char('a'),
        KeyCode::Char('é'),
        KeyCode::Char('🦀'),
        KeyCode::Enter,
        KeyCode::Backspace,
        KeyCode::Tab,
        KeyCode::Escape,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::BackTab,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Insert,
        KeyCode::Delete,
        KeyCode::KeypadBegin,
        KeyCode::CapsLock,
        KeyCode::ScrollLock,
        KeyCode::NumLock,
        KeyCode::PrintScreen,
        KeyCode::Pause,
        KeyCode::Menu,
        KeyCode::Null,
        KeyCode::Function(1),
        KeyCode::Function(35),
    ]
    .into_iter()
    .chain(
        [
            Modifier::LeftShift,
            Modifier::LeftControl,
            Modifier::LeftAlt,
            Modifier::LeftSuper,
            Modifier::LeftHyper,
            Modifier::LeftMeta,
            Modifier::RightShift,
            Modifier::RightControl,
            Modifier::RightAlt,
            Modifier::RightSuper,
            Modifier::RightHyper,
            Modifier::RightMeta,
            Modifier::IsoLevel3Shift,
            Modifier::IsoLevel5Shift,
        ]
        .map(KeyCode::Modifier),
    )
    .chain(
        [
            Media::Play,
            Media::Pause,
            Media::PlayPause,
            Media::Reverse,
            Media::Stop,
            Media::FastForward,
            Media::Rewind,
            Media::TrackNext,
            Media::TrackPrevious,
            Media::Record,
            Media::LowerVolume,
            Media::RaiseVolume,
            Media::MuteVolume,
        ]
        .map(KeyCode::Media),
    );
    for code in codes {
        let key = KeyEvent::from(code);
        assert_eq!(round_trip(key), key);
    }
}

#[test]
fn every_kind_modifier_and_state_bit_survives_saving() {
    for kind in [
        KeyEventKind::Press,
        KeyEventKind::Release,
        KeyEventKind::Repeat,
    ] {
        for bits in [0, 1, 1 << 3, 1 << 7, u8::MAX] {
            let key = KeyEvent {
                code: KeyCode::Char('x'),
                kind,
                modifiers: Modifiers::from_bits_retain(bits),
                state: KeyEventState::from_bits_retain(bits),
            };
            assert_eq!(round_trip(key), key);
        }
    }
}
