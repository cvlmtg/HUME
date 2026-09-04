//! Whether [`Editor::run`](super::Editor::run)'s event loop owns the
//! terminal, and the handle to drive it when it does.
//!
//! One field, not two, because the pair has an illegal combination — active
//! with no handle — that only prose could rule out if "is the loop running"
//! and "is there a terminal" were tracked separately. Folded together, the
//! type rules it out instead. [`ActiveTui`] carries that same guarantee past
//! this type's own lifetime: a value [`Tui::as_active`] produces stays a
//! `Tui` minus `Off` for as long as `InlineOutput` holds onto it, so code
//! reading a pushed/entered bracket never has to ask a *different* `Tui` (a
//! different `Editor`/`EditorHostImpl`'s current one) whether the fact it
//! captured is still true. Unlike [`InlineOutput`](super::InlineOutput),
//! whose module doc explains why *its* two facts must stay separate fields,
//! nothing here ever needs one of these facts without the other.

use hume_platform::terminal::SharedTerm;

/// `Clone` so [`EditorHostImpl`](super::host_impl::EditorHostImpl) can hold
/// one by value: `EditorHostImpl::new` has no `Editor` to borrow `tui` from
/// at all (it builds a host for callers with no terminal/`OutputHost` need),
/// so a `&'a Tui` field would need a file-scope `static Tui::Off` to point
/// at instead. Cloning costs two `Arc` bumps (`SharedTerm` is `Arc` +
/// `EventReader`, both cheap to clone — see its own doc in
/// `hume_platform::terminal`) against that alternative's global state.
#[derive(Clone)]
pub(crate) enum Tui {
    /// [`Editor::run`](super::Editor::run) does not own the terminal: before
    /// and after the event loop, plus tests and headless `run_keys`.
    Off,
    /// `Editor::run`'s event loop owns this handle.
    On(SharedTerm),
    /// Test-only: event-loop semantics with no TTY attached — the shape
    /// `Editor::run` never actually produces, used by tests that need
    /// [`Self::as_active`] to return `Some` without a real terminal to
    /// drive.
    #[cfg(test)]
    OnHeadless,
}

impl Tui {
    /// The terminal handle, if there is one — makes no claim about whether
    /// the event loop is active. Safe to call from code that can legitimately
    /// run with no terminal attached at all (e.g. `resync_mouse_mode`).
    pub(crate) fn terminal(&self) -> Option<&SharedTerm> {
        match self {
            Tui::On(term) => Some(term),
            #[cfg(test)]
            Tui::OnHeadless => None,
            Tui::Off => None,
        }
    }

    /// This `Tui`'s active handle, captured by value — `Off` becomes `None`;
    /// `On`/`OnHeadless` clone into the narrower [`ActiveTui`] shape a pushed
    /// `InlineOutput` frame carries forward. See [`ActiveTui`]'s own doc for
    /// why capturing this rather than re-reading `tui` later matters.
    pub(crate) fn as_active(&self) -> Option<ActiveTui> {
        match self {
            Tui::Off => None,
            Tui::On(term) => Some(ActiveTui::On(term.clone())),
            #[cfg(test)]
            Tui::OnHeadless => Some(ActiveTui::Headless),
        }
    }
}

/// A [`Tui`] known, at the point this was captured, to have been active —
/// `Tui` minus `Off`. `InlineOutput::Frame` captures one via
/// [`Tui::as_active`] at push time and `Entered` captures the same value
/// again at `mark_entered`, so `ensure_inline_output_screen`/
/// `close_inline_output_bracket` read what was true when the bracket was
/// armed/entered, never a fresh `Editor`/`EditorHostImpl::tui` that may
/// belong to a different host than the one that pushed the frame.
#[derive(Clone)]
pub(crate) enum ActiveTui {
    On(SharedTerm),
    /// Test-only twin of [`Tui::OnHeadless`] — see that variant's doc.
    #[cfg(test)]
    Headless,
}

impl ActiveTui {
    /// The terminal handle — `None` only for the test-only headless shape.
    pub(crate) fn terminal(&self) -> Option<&SharedTerm> {
        match self {
            ActiveTui::On(term) => Some(term),
            #[cfg(test)]
            ActiveTui::Headless => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_has_no_active_handle() {
        assert!(Tui::Off.terminal().is_none());
        assert!(Tui::Off.as_active().is_none());
    }

    #[test]
    fn headless_converts_to_an_active_headless_handle() {
        let active = Tui::OnHeadless.as_active().expect("OnHeadless is active");
        assert!(active.terminal().is_none());
    }
}
