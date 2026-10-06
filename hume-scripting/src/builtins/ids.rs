//! Opaque Steel types for buffer/pane identity.
//!
//! [`SteelPane`] is the crossing-boundary value: every command, hook,
//! completion source, and builtin argument or return that names a buffer
//! carries a `PaneHandle` wrapped in `SteelPane`. Plugins receive and pass
//! these between builtins but cannot construct or inspect them
//! arithmetically; they are purely opaque handles. [`SteelBufferKey`] is
//! `(buffer-key pane)`'s own return type: a per-buffer hash/comparison key,
//! undecodable by [`super::args::ArgPane`]/[`super::args::LivePane`]
//! so a key can't be passed back into a builtin expecting a pane. A pane
//! value and a plain per-buffer key must stay two distinct kinds of thing,
//! not the same value with its pane field cleared. Collapsing them would
//! let a key masquerade as a pane-less handle anywhere a pane is expected,
//! silently reintroducing the implicit-pane guessing this design exists to
//! remove (see `types::PaneHandle`'s own doc).
//!
//! Display uses the slotmap `as_ffi` u64 so that `(log! "info" pane)` prints
//! something readable without revealing internal structure.

use hume_editing::text::TextVersion;
use hume_engine::pipeline::BufferId;
use slotmap::Key as _;
use steel::{
    gc::ShareableMut as _,
    rvals::{Custom, IntoSteelVal as _, SteelVal, as_underlying_type},
};

use hume_lsp::backend::ServerId;
use hume_rope::offset::CharOffset;

use crate::types::{PaneHandle, ServerName};

// ── Wrapper types ─────────────────────────────────────────────────────────────

/// Opaque Steel handle for a [`PaneHandle`]; see this module's own doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SteelPane(pub(crate) PaneHandle);

impl SteelPane {
    /// Wrap a `PaneHandle` into a Steel-facing opaque handle.
    pub fn new(handle: PaneHandle) -> Self {
        Self(handle)
    }

    /// Convert to a `SteelVal` without returning `Result`.
    ///
    /// `IntoSteelVal` for custom types is infallible; this avoids `.expect()` at
    /// every call site that wraps a `PaneHandle` for hook args or builtin returns.
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("SteelPane into_steelval")
    }
}

/// `(buffer-key pane)`'s return: `pane`'s buffer, with no pane component.
/// See this module's own doc for why this is a distinct type rather than a
/// [`SteelPane`] with its pane field cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SteelBufferKey(pub(crate) BufferId);

impl Custom for SteelPane {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(match self.0.pane() {
            Some(pid) => format!(
                "#<pane buffer={} pane={}>",
                self.0.buffer().data().as_ffi(),
                pid.data().as_ffi()
            ),
            None => format!("#<pane buffer={}>", self.0.buffer().data().as_ffi()),
        }))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o.0 == self.0)
    }

    fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
        Some(self)
    }
}

impl Custom for SteelBufferKey {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<buffer-key {}>", self.0.data().as_ffi())))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o.0 == self.0)
    }

    fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
        Some(self)
    }
}

/// Opaque Steel value for one running language-server process. Equality
/// and hashing use `id` alone: a server that stops or restarts is a
/// different process with a different id, so a value kept across a restart
/// never names its successor.
#[derive(Debug, Clone)]
pub struct ServerRef {
    pub id: ServerId,
    pub name: ServerName,
}

impl ServerRef {
    /// Convert to a `SteelVal` without returning `Result`; see
    /// [`SteelPane::into_steel_val`].
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("ServerRef into_steelval")
    }

    /// The server `val` holds, or `None` if it is some other value.
    pub fn from_steel_val(val: &SteelVal) -> Option<Self> {
        downcast(val)
    }
}

impl PartialEq for ServerRef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for ServerRef {}

impl std::hash::Hash for ServerRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Custom for ServerRef {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<lsp-server {}>", self.name)))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o == self)
    }

    fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
        Some(self)
    }
}

/// A buffer position in a request's params, encoded as a wire `Position`
/// only when the request is serialized for a server, in that server's
/// position encoding. `offset` is a char offset into the text of `buffer`
/// that was at `version` when the value was built; it means nothing for any
/// other version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocPos {
    pub buffer: BufferId,
    pub version: TextVersion,
    pub offset: CharOffset,
}

/// The [`DocPos`] counterpart for a wire `Range`: half-open `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocRange {
    pub buffer: BufferId,
    pub version: TextVersion,
    pub start: CharOffset,
    pub end: CharOffset,
}

impl DocPos {
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("DocPos into_steelval")
    }

    pub fn from_steel_val(val: &SteelVal) -> Option<Self> {
        downcast(val)
    }
}

impl DocRange {
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("DocRange into_steelval")
    }

    pub fn from_steel_val(val: &SteelVal) -> Option<Self> {
        downcast(val)
    }
}

impl Custom for DocPos {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!(
            "#<doc-pos buffer={} offset={}>",
            self.buffer.data().as_ffi(),
            self.offset.index()
        )))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o == self)
    }
}

impl Custom for DocRange {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!(
            "#<doc-range buffer={} {}..{}>",
            self.buffer.data().as_ffi(),
            self.start.index(),
            self.end.index()
        )))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o == self)
    }
}

/// The `T` a Steel custom value holds, cloned out, or `None` for any other
/// value.
pub(crate) fn downcast<T: Clone + 'static>(val: &SteelVal) -> Option<T> {
    if let SteelVal::Custom(v) = val {
        v.read().as_any_ref().downcast_ref::<T>().cloned()
    } else {
        None
    }
}

// ── Predicate builtins ────────────────────────────────────────────────────────

/// `(pane? v)`: return `#t` if `v` is an opaque pane handle.
pub(crate) fn is_pane(val: SteelVal) -> bool {
    downcast_pane(&val).is_some()
}

// ── Decode ────────────────────────────────────────────────────────────────────

pub(crate) fn downcast_pane(val: &SteelVal) -> Option<PaneHandle> {
    downcast::<SteelPane>(val).map(|p| p.0)
}

#[cfg(test)]
mod tests;
