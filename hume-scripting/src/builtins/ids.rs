//! Opaque Steel types for buffer/pane identity.
//!
//! [`SteelPane`] is the crossing-boundary value: every command, hook,
//! completion source, and builtin argument or return that names a buffer
//! carries a `PaneHandle` wrapped in `SteelPane`. Plugins receive and pass
//! these between builtins but cannot construct or inspect them
//! arithmetically; they are purely opaque handles. [`SteelBufferKey`] is
//! `(buffer-key pane)`'s own return type: a per-buffer hash/comparison key,
//! undecodable by every pane argument type ([`super::args::pane`])
//! so a key can't be passed back into a builtin expecting a pane. A pane
//! value and a plain per-buffer key must stay two distinct kinds of thing,
//! not the same value with its pane field cleared. Collapsing them would
//! let a key masquerade as a pane-less handle anywhere a pane is expected,
//! bringing back the implicit-pane guessing [`hume_core::PaneHandle`] exists
//! to remove.
//!
//! Display uses the slotmap `as_ffi` u64 so that `(log! "info" pane)` prints
//! something readable without revealing internal structure.

use hume_core::{BufferId, PaneHandle};
use hume_editing::text::TextVersion;
use slotmap::Key as _;
use steel::{
    gc::ShareableMut as _,
    rvals::{Custom, IntoSteelVal as _, SteelVal, as_underlying_type},
};

use hume_lsp::backend::ServerId;
use hume_rope::offset::CharOffset;

use crate::host::{JobHandle, SupersedeKey, TimerHandle};
use hume_core::ServerName;

// ── Wrapper types ─────────────────────────────────────────────────────────────

/// The Steel surface of a value that compares by its `PartialEq`, hashes by
/// its `Hash` and prints as `$fmt`, with `$v` bound to the value. The
/// `fn into_steel_val` form also gives the type an `into_steel_val` that
/// returns a `SteelVal` without a `Result`: `IntoSteelVal` for a custom type
/// is infallible, so call sites wrapping one for a hook arg or builtin return
/// need no `.expect()` of their own.
macro_rules! steel_value {
    ($vis:vis fn into_steel_val for $ty:ident => |$v:ident| $fmt:expr) => {
        impl $ty {
            $vis fn into_steel_val(self) -> SteelVal {
                self.into_steelval()
                    .expect(concat!(stringify!($ty), " into_steelval"))
            }
        }

        steel_value!($ty => |$v| $fmt);
    };
    ($ty:ident => |$v:ident| $fmt:expr) => {
        impl Custom for $ty {
            fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
                let $v = self;
                Some(Ok($fmt))
            }

            fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
                as_underlying_type::<Self>(other).is_some_and(|o| o == self)
            }

            fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
                Some(self)
            }
        }
    };
}

/// Opaque Steel handle for a [`PaneHandle`]; see this module's own doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SteelPane(pub(crate) PaneHandle);

impl SteelPane {
    /// Wrap a `PaneHandle` into a Steel-facing opaque handle.
    pub fn new(handle: PaneHandle) -> Self {
        Self(handle)
    }
}

/// A pane, or `#f` when there is none.
pub(crate) fn pane_or_false(pane: Option<PaneHandle>) -> SteelVal {
    pane.map_or(SteelVal::BoolV(false), |pane| {
        SteelPane(pane).into_steel_val()
    })
}

/// `(buffer-key pane)`'s return: `pane`'s buffer, with no pane component.
/// See this module's own doc for why this is a distinct type rather than a
/// [`SteelPane`] with its pane field cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SteelBufferKey(pub(crate) BufferId);

steel_value!(pub fn into_steel_val for SteelPane => |p| match p.0.pane() {
    Some(pid) => format!(
        "#<pane buffer={} pane={}>",
        p.0.buffer().data().as_ffi(),
        pid.data().as_ffi()
    ),
    None => format!("#<pane buffer={}>", p.0.buffer().data().as_ffi()),
});

steel_value!(SteelBufferKey => |k| format!("#<buffer-key {}>", k.0.data().as_ffi()));

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

steel_value!(pub fn into_steel_val for ServerRef => |s| format!("#<lsp-server {}>", s.name));

steel_value!(pub(crate) fn into_steel_val for TimerHandle => |h| format!("#<timer {}>", h.raw()));
steel_value!(pub(crate) fn into_steel_val for JobHandle => |h| format!("#<job {}>", h.raw()));
steel_value!(
    pub(crate) fn into_steel_val for SupersedeKey => |k| format!("#<supersede-key {}>", k.raw())
);

/// `(buffer-generation pane)`'s value: the generation of `buffer`'s text
/// when it was read. It names its buffer because generations are counted
/// per buffer, so the same number from two buffers names two unrelated
/// texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BufferGeneration {
    pub(crate) buffer: BufferId,
    pub(crate) generation: u64,
}

impl BufferGeneration {
    /// The generation `val` holds for `buffer`. `arg` names the argument in
    /// the error for any other value, a generation of another buffer included.
    pub(crate) fn of(val: &SteelVal, buffer: BufferId, arg: &str) -> Result<u64, String> {
        match downcast::<Self>(val) {
            Some(g) if g.buffer == buffer => Ok(g.generation),
            Some(_) => Err(format!("{arg}: a generation of another buffer")),
            None => Err(format!("{arg}: expected a value from buffer-generation")),
        }
    }
}

steel_value!(pub(crate) fn into_steel_val for BufferGeneration => |g| format!(
    "#<buffer-generation buffer={} {}>",
    g.buffer.data().as_ffi(),
    g.generation
));

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
