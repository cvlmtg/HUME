//! Opaque keys for the buffers and panes the editor owns, the pairing of a
//! buffer with the pane it was reached through, and the token naming an
//! object the editor keeps.

use std::sync::atomic::{AtomicU64, Ordering};

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
/// Callers pair a buffer only with a pane that showed it. Nothing here
/// checks that, and the pane may have moved to another buffer since, so a
/// consumer that needs the pane to show the buffer checks it on use.
///
/// `pane` is `None` for a value with no pane of its own (a buffer-level hook
/// argument, `(buffers)`'s list, `open-buffer!`'s return). A consumer that
/// needs pane state (selections, viewport, focus) fails fast on `None`
/// rather than guessing one; one that needs only the buffer
/// ([`Self::buffer`]) works the same either way. A text edit that accepts
/// either acts through the pane while it shows the buffer and through no
/// pane otherwise.
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
/// position, a held write, a completion request and its answer), held by a
/// script or by a reply the editor awaits. It scopes every later call to the
/// instance that minted it: a late call naming an instance that is gone names
/// a token nothing holds, and reads as a no-op. A script holds it as an
/// integer.
///
/// Every kind mints from one process-wide counter, so no two minted tokens
/// are equal, and a token one kind minted never names another kind's live
/// object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostToken(u64);

static NEXT: AtomicU64 = AtomicU64::new(1);

impl HostToken {
    /// Names nothing: what a `#f` token argument decodes to. [`Self::mint`]
    /// starts at `1`, so no minted token equals it.
    pub const NONE: Self = Self(0);

    /// A token no other call has returned.
    pub fn mint() -> Self {
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// Wraps an integer a script handed back. Any value is safe to wrap: a
    /// forged one matches at most a live object of the kind it is handed to,
    /// never a different kind.
    pub fn from_script(raw: u64) -> Self {
        Self(raw)
    }

    /// The integer a script holds.
    pub fn raw(self) -> u64 {
        self.0
    }
}
