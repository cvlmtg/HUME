//! Opaque keys for the buffers and panes the editor owns, the pairing of a
//! buffer with the pane it was reached through, and the token naming an
//! object the editor keeps.

use slotmap::new_key_type;

new_key_type! {
    /// Opaque handle to a buffer.
    pub struct BufferId;
    /// Opaque handle to a pane.
    pub struct PaneId;
}

/// A buffer paired with the pane it was invoked or observed through, if
/// any: the target a command runs against, and the value hooks, completion
/// sources and every buffer-taking builtin name a buffer by.
///
/// `pane` is `None` for a value with no pane of its own (a buffer-level hook
/// argument, `(buffers)`'s list, `open-buffer!`'s return). A consumer that
/// needs pane state (selections, viewport, focus) fails fast on `None`
/// rather than guessing one; one that needs only the buffer
/// ([`Self::buffer`]) works the same either way. A text edit acts through
/// the pane when the value carries one and through no pane otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneHandle {
    buffer: BufferId,
    pane: Option<PaneId>,
}

impl PaneHandle {
    /// A buffer observed or invoked through a specific pane.
    pub fn with_pane(buffer: BufferId, pane: PaneId) -> Self {
        Self {
            buffer,
            pane: Some(pane),
        }
    }

    /// A buffer with no pane of its own.
    pub fn buffer_only(buffer: BufferId) -> Self {
        Self { buffer, pane: None }
    }

    pub fn buffer(self) -> BufferId {
        self.buffer
    }

    pub fn pane(self) -> Option<PaneId> {
        self.pane
    }
}

/// Opaque handle for an object the editor keeps (an open widget, a tracked
/// position), scoping every later call to the instance that minted it: a
/// late call naming an instance that is gone names a token nothing holds,
/// and reads as a no-op. A script holds it as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostToken(u64);

impl HostToken {
    /// Names nothing: what a `#f` token argument decodes to. The editor's
    /// mint starts at `1`, so no live object holds it.
    pub const NONE: Self = Self(0);

    /// Wraps a counter value the editor minted, or an integer a script
    /// handed back. Any value is safe to wrap: a forged one matches at most a
    /// live object of the kind it is handed to, never a different kind.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The integer a script holds.
    pub fn raw(self) -> u64 {
        self.0
    }
}
