//! A recorded key as plain data. termina's key types have no serde support,
//! so each is mirrored here with a conversion both ways; the matches are
//! exhaustive, so a key termina adds is a compile error until it is mirrored.

use serde::{Deserialize, Serialize};
use termina::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, MediaKeyCode, ModifierKeyCode, Modifiers,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SavedKey {
    code: SavedCode,
    kind: SavedKind,
    /// [`Modifiers`]' bits.
    modifiers: u8,
    /// [`KeyEventState`]'s bits.
    state: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedCode {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    BackTab,
    PageUp,
    PageDown,
    Insert,
    Delete,
    KeypadBegin,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
    Null,
    Function(u8),
    Modifier(SavedModifier),
    Media(SavedMedia),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedKind {
    Press,
    Release,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedModifier {
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedMedia {
    Play,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

impl From<KeyEvent> for SavedKey {
    fn from(key: KeyEvent) -> Self {
        Self {
            code: key.code.into(),
            kind: key.kind.into(),
            modifiers: key.modifiers.bits(),
            state: key.state.bits(),
        }
    }
}

impl From<SavedKey> for KeyEvent {
    fn from(key: SavedKey) -> Self {
        Self {
            code: key.code.into(),
            kind: key.kind.into(),
            modifiers: Modifiers::from_bits_retain(key.modifiers),
            state: KeyEventState::from_bits_retain(key.state),
        }
    }
}

/// `From` both ways between a termina enum whose variants carry nothing and
/// its mirror.
macro_rules! mirror {
    ($theirs:ident <=> $ours:ident { $($variant:ident),* $(,)? }) => {
        impl From<$theirs> for $ours {
            fn from(value: $theirs) -> Self {
                match value {
                    $($theirs::$variant => Self::$variant,)*
                }
            }
        }

        impl From<$ours> for $theirs {
            fn from(value: $ours) -> Self {
                match value {
                    $($ours::$variant => Self::$variant,)*
                }
            }
        }
    };
}

mirror!(KeyEventKind <=> SavedKind { Press, Release, Repeat });
mirror!(ModifierKeyCode <=> SavedModifier {
    LeftShift, LeftControl, LeftAlt, LeftSuper, LeftHyper, LeftMeta, RightShift,
    RightControl, RightAlt, RightSuper, RightHyper, RightMeta, IsoLevel3Shift, IsoLevel5Shift,
});
mirror!(MediaKeyCode <=> SavedMedia {
    Play, Pause, PlayPause, Reverse, Stop, FastForward, Rewind, TrackNext, TrackPrevious,
    Record, LowerVolume, RaiseVolume, MuteVolume,
});

impl From<KeyCode> for SavedCode {
    fn from(code: KeyCode) -> Self {
        match code {
            KeyCode::Char(c) => Self::Char(c),
            KeyCode::Enter => Self::Enter,
            KeyCode::Backspace => Self::Backspace,
            KeyCode::Tab => Self::Tab,
            KeyCode::Escape => Self::Escape,
            KeyCode::Left => Self::Left,
            KeyCode::Right => Self::Right,
            KeyCode::Up => Self::Up,
            KeyCode::Down => Self::Down,
            KeyCode::Home => Self::Home,
            KeyCode::End => Self::End,
            KeyCode::BackTab => Self::BackTab,
            KeyCode::PageUp => Self::PageUp,
            KeyCode::PageDown => Self::PageDown,
            KeyCode::Insert => Self::Insert,
            KeyCode::Delete => Self::Delete,
            KeyCode::KeypadBegin => Self::KeypadBegin,
            KeyCode::CapsLock => Self::CapsLock,
            KeyCode::ScrollLock => Self::ScrollLock,
            KeyCode::NumLock => Self::NumLock,
            KeyCode::PrintScreen => Self::PrintScreen,
            KeyCode::Pause => Self::Pause,
            KeyCode::Menu => Self::Menu,
            KeyCode::Null => Self::Null,
            KeyCode::Function(n) => Self::Function(n),
            KeyCode::Modifier(m) => Self::Modifier(m.into()),
            KeyCode::Media(m) => Self::Media(m.into()),
        }
    }
}

impl From<SavedCode> for KeyCode {
    fn from(code: SavedCode) -> Self {
        match code {
            SavedCode::Char(c) => Self::Char(c),
            SavedCode::Enter => Self::Enter,
            SavedCode::Backspace => Self::Backspace,
            SavedCode::Tab => Self::Tab,
            SavedCode::Escape => Self::Escape,
            SavedCode::Left => Self::Left,
            SavedCode::Right => Self::Right,
            SavedCode::Up => Self::Up,
            SavedCode::Down => Self::Down,
            SavedCode::Home => Self::Home,
            SavedCode::End => Self::End,
            SavedCode::BackTab => Self::BackTab,
            SavedCode::PageUp => Self::PageUp,
            SavedCode::PageDown => Self::PageDown,
            SavedCode::Insert => Self::Insert,
            SavedCode::Delete => Self::Delete,
            SavedCode::KeypadBegin => Self::KeypadBegin,
            SavedCode::CapsLock => Self::CapsLock,
            SavedCode::ScrollLock => Self::ScrollLock,
            SavedCode::NumLock => Self::NumLock,
            SavedCode::PrintScreen => Self::PrintScreen,
            SavedCode::Pause => Self::Pause,
            SavedCode::Menu => Self::Menu,
            SavedCode::Null => Self::Null,
            SavedCode::Function(n) => Self::Function(n),
            SavedCode::Modifier(m) => Self::Modifier(m.into()),
            SavedCode::Media(m) => Self::Media(m.into()),
        }
    }
}
