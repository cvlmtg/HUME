//! The pane-value argument types of the `builtins!` table. Each names what a
//! builtin needs from a pane value:
//!
//! | Type | Needs ([`PaneNeed`]) | Resolves to |
//! |---|---|---|
//! | [`LiveBuffer`] | the value's buffer open | its [`BufferId`] |
//! | [`RequesterPane`] | the buffer open; the pane half is a hint | the [`PaneHandle`] |
//! | [`ShownPane`] | a pane that still shows the buffer | a [`ShownHandle`] |
//! | [`FocusedPane`] | that pane focused too | a [`FocusedHandle`] |
//!
//! Every [`BuiltinArg::resolve`] checks that the buffer is open. The host
//! checks a [`ShownHandle`] or [`FocusedHandle`] when a host method acts
//! through it, so a body reports the arguments it parses before the pane is
//! checked. `clippy.toml` bans reading a handle's raw pane anywhere but
//! that check. A host method that acts through a pane takes one of these
//! handles, and outside `cfg(test)` and the `test-util` feature only these
//! resolves make one, so a builtin reaches such a method only by declaring
//! the matching type. [`ArgPane`] checks nothing, for a builtin that answers
//! for a closed buffer.

use hume_core::{BufferId, PaneHandle};
use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{FromSteelVal, SteelVal};

use super::{BuiltinArg, not_live_err};

/// What a builtin needs from a pane value, in increasing order of demand:
/// its buffer open, a pane hint beside an open buffer, a pane that still
/// shows that buffer, or the focused pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaneNeed {
    Buffer,
    Requester,
    Shown,
    Focused,
}

fn decode(val: &SteelVal) -> Result<PaneHandle, SteelErr> {
    crate::builtins::ids::downcast_pane(val)
        .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "expected pane".to_string()))
}

/// `handle`, if its buffer is open. Every type below checks this first, so a
/// closed buffer reads the same whatever the builtin needs.
fn live(
    handle: PaneHandle,
    ctx: &mut crate::SteelCtx,
    arg: &'static str,
) -> Result<PaneHandle, SteelErr> {
    if ctx.host.buffers().buffer_exists(handle.buffer()) {
        Ok(handle)
    } else {
        Err(not_live_err(arg, handle.buffer()))
    }
}

macro_rules! pane_arg {
    ($($(#[$doc:meta])* $name:ident($vis:vis PaneHandle)),+ $(,)?) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy)]
            pub(crate) struct $name($vis PaneHandle);

            impl FromSteelVal for $name {
                fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
                    decode(val).map($name)
                }
            }
        )+
    };
}

pane_arg! {
    /// A pane value whose buffer the builtin acts on alone.
    LiveBuffer(PaneHandle),
    /// A pane value whose buffer the builtin acts on, through its pane
    /// while that pane shows the buffer and through no pane otherwise.
    RequesterPane(PaneHandle),
    /// A pane value the builtin checks nothing of, for one that answers for a
    /// closed buffer the way it answers for a buffer with nothing to report
    /// (`diagnostic-counts`, `buffer-live?`, …).
    ArgPane(pub(crate) PaneHandle),
}

impl BuiltinArg for LiveBuffer {
    const PANE_NEED: Option<PaneNeed> = Some(PaneNeed::Buffer);
    type Out = BufferId;
    fn resolve(self, ctx: &mut crate::SteelCtx, arg: &'static str) -> Result<BufferId, SteelErr> {
        live(self.0, ctx, arg).map(PaneHandle::buffer)
    }
}

impl BuiltinArg for RequesterPane {
    const PANE_NEED: Option<PaneNeed> = Some(PaneNeed::Requester);
    type Out = PaneHandle;
    fn resolve(self, ctx: &mut crate::SteelCtx, arg: &'static str) -> Result<PaneHandle, SteelErr> {
        live(self.0, ctx, arg)
    }
}

/// An argument type whose resolve makes the handle a host method acting
/// through the pane takes.
macro_rules! acting_pane_arg {
    ($($(#[$arg_doc:meta])* $arg:ident($need:ident) => $(#[$handle_doc:meta])* $handle:ident),+ $(,)?) => {
        pane_arg! { $($(#[$arg_doc])* $arg(PaneHandle)),+ }

        $(
            impl BuiltinArg for $arg {
                const PANE_NEED: Option<PaneNeed> = Some(PaneNeed::$need);
                type Out = $handle;
                fn resolve(
                    self,
                    ctx: &mut crate::SteelCtx,
                    arg: &'static str,
                ) -> Result<$handle, SteelErr> {
                    live(self.0, ctx, arg).map($handle)
                }
            }

            $(#[$handle_doc])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $handle(PaneHandle);

            impl $handle {
                /// `handle` as a builtin argument would resolve it, for a
                /// test that calls a host method directly.
                #[cfg(any(test, feature = "test-util"))]
                pub fn for_test(handle: PaneHandle) -> Self {
                    Self(handle)
                }

                pub fn handle(self) -> PaneHandle {
                    self.0
                }
            }
        )+
    };
}

acting_pane_arg! {
    /// A pane value the builtin acts through, which must still show its
    /// buffer.
    ShownPane(Shown) =>
    /// A pane value from a `ShownPane` argument, whose buffer was open when
    /// it resolved. The host checks that its pane still shows that buffer
    /// before acting through it.
    ShownHandle,
    /// A pane value the builtin acts through, which must still show its
    /// buffer and be the focused pane.
    FocusedPane(Focused) =>
    /// A pane value from a `FocusedPane` argument, whose buffer was open when
    /// it resolved. The host checks that its pane still shows that buffer
    /// and is focused before acting through it.
    FocusedHandle,
}

#[cfg(test)]
mod tests;
