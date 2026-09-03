//! State of the `#:inline-output` terminal bracket for the command(s)
//! currently on the Steel call stack.
//!
//! Two independent facts, tracked as separate fields rather than folded into
//! one enum:
//!
//! - *A declared `#:inline-output` command is on the Steel call stack.*
//!   Nests one frame per `call!` into a declared command, pushed by
//!   [`InlineOutput::push`] and popped by [`InlineOutput::pop`] (the Rust
//!   side of `%restore-inline-output!`) or dropped wholesale by
//!   [`InlineOutput::drain_frames`] — the backstop for a frame a caught
//!   Steel error left unpaired, and also how the top-level dispatch's own
//!   frame closes, since nothing calls a matching `pop` for it.
//! - *The alt-screen has actually been left.* Happens at most once per
//!   dispatch no matter how deep the `call!` nesting goes — entering twice
//!   would pop the kitty keyboard-protocol stack twice for one push — and
//!   can only be undone at the Rust boundary that owns the terminal
//!   (`Editor::close_inline_output_bracket`), long after whichever Steel
//!   frame caused it has already returned. Tracked by `entered`, which frame
//!   pops never touch.
//!
//! `ran` records whether any frame this session actually owned the terminal
//! (`tui_active` was true when it was pushed) — survives frame pops for the
//! same reason `entered` does, so the disk-change sweep a subprocess may
//! warrant still fires even for a command that armed but never printed.

/// One `#:inline-output` command currently on the Steel call stack.
struct Frame {
    /// Printed in the running banner on first output.
    name: String,
    /// Whether `Editor::run` owned the terminal when this frame was pushed —
    /// the old `Armed`/`Headless` split. `false` means there is no alt-screen
    /// to leave for this frame (tests, headless `run_keys`); the stdout gate
    /// still opens (raw writes are safe with no TUI to protect), but
    /// [`InlineOutput::needs_enter`] never fires for it.
    tui: bool,
}

/// Terminal state captured when the alt-screen was actually left, so
/// `Editor::close_inline_output_bracket` restores exactly what it saw rather
/// than re-reading `Editor`/`EditorSettings` fields that may have changed
/// mid-command (`:set global mouse-enabled=…` inside the very body that's
/// running, say).
pub(crate) struct Entered {
    pub(crate) kitty: bool,
    pub(crate) mouse: bool,
    pub(crate) mouse_select: bool,
}

/// See the module doc for the two facts this tracks and why they're separate
/// fields rather than one enum.
#[derive(Default)]
pub(crate) struct InlineOutput {
    frames: Vec<Frame>,
    entered: Option<Entered>,
    ran: bool,
    /// Test-only seam: counts every real [`Self::mark_entered`] this
    /// `Editor`'s lifetime has made, so a test can assert the bracket fired
    /// at least once without capturing real terminal I/O. Unlike `entered`,
    /// never reset — a session boundary clearing it would defeat the point.
    #[cfg(test)]
    enters: usize,
}

impl InlineOutput {
    /// Push a frame for `name`, currently executing with terminal ownership
    /// `tui`. Marks `ran` when `tui` — see the module doc.
    pub(crate) fn push(&mut self, name: &str, tui: bool) {
        if tui {
            self.ran = true;
        }
        self.frames.push(Frame {
            name: name.to_string(),
            tui,
        });
    }

    /// Pop the innermost frame — the Rust side of a `call!`-armed nested
    /// command's matching `%restore-inline-output!`. Never touches `entered`
    /// or `ran`: an inner command's frame closing must not strand the outer
    /// command still on the stack above it.
    pub(crate) fn pop(&mut self) {
        self.frames.pop();
    }

    /// Drop every outstanding frame. `entered`/`ran` survive — those are
    /// read at the Rust boundary right after this runs.
    pub(crate) fn drain_frames(&mut self) {
        self.frames.clear();
    }

    /// Whether a declared `#:inline-output` command is anywhere on the
    /// current call stack — the stdout gate opens whenever this is true,
    /// with or without a live alt-screen to protect.
    pub(crate) fn is_open(&self) -> bool {
        !self.frames.is_empty()
    }

    /// The innermost frame's name if it owns the terminal and the alt-screen
    /// hasn't already been left this session — `None` otherwise, including
    /// once `entered` is already set, so nesting deeper after the first real
    /// print can never re-enter.
    pub(crate) fn needs_enter(&self) -> Option<&str> {
        if self.entered.is_some() {
            return None;
        }
        self.frames
            .last()
            .filter(|frame| frame.tui)
            .map(|frame| frame.name.as_str())
    }

    /// Record that the alt-screen was actually left, for this session.
    pub(crate) fn mark_entered(&mut self, kitty: bool, mouse: bool, mouse_select: bool) {
        self.entered = Some(Entered {
            kitty,
            mouse,
            mouse_select,
        });
        #[cfg(test)]
        {
            self.enters += 1;
        }
    }

    /// Take the terminal state saved by [`Self::mark_entered`], clearing it —
    /// call once, at the Rust boundary closing the bracket.
    pub(crate) fn take_entered(&mut self) -> Option<Entered> {
        self.entered.take()
    }

    /// Take (and clear) whether any frame this session owned the terminal —
    /// call once, at the same Rust boundary as [`Self::take_entered`].
    pub(crate) fn take_ran(&mut self) -> bool {
        std::mem::take(&mut self.ran)
    }

    /// Test-only seam: whether the alt-screen has ever actually been left
    /// during this `Editor`'s lifetime — see `enters`' own doc.
    #[cfg(test)]
    pub(crate) fn ever_entered(&self) -> bool {
        self.enters > 0
    }

    /// Test-only seam: how many times the alt-screen has actually been left
    /// during this `Editor`'s lifetime — lets a test pin an exact count (a
    /// re-entry bug shows up as `2`, not just "entered"), same lifetime as
    /// `ever_entered`.
    #[cfg(test)]
    pub(crate) fn enter_count(&self) -> usize {
        self.enters
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_open_tracks_frame_count_through_nesting() {
        let mut io = InlineOutput::default();
        assert!(!io.is_open());
        io.push("outer", true);
        assert!(io.is_open());
        io.push("inner", true);
        assert!(io.is_open());
        io.pop();
        assert!(io.is_open(), "outer frame must still be on the stack");
        io.pop();
        assert!(!io.is_open());
    }

    #[test]
    fn needs_enter_fires_once_per_session_regardless_of_nesting() {
        let mut io = InlineOutput::default();
        io.push("outer", true);
        assert_eq!(io.needs_enter(), Some("outer"));
        io.mark_entered(false, false, false);
        assert_eq!(io.needs_enter(), None);
        // A nested call! after the outer already entered must not re-enter.
        io.push("inner", true);
        assert_eq!(io.needs_enter(), None, "already entered this session");
    }

    #[test]
    fn needs_enter_skips_a_headless_frame() {
        let mut io = InlineOutput::default();
        io.push("cmd", false);
        assert!(io.is_open(), "gate still opens off the event loop");
        assert_eq!(io.needs_enter(), None);
    }

    #[test]
    fn drain_frames_preserves_entered_and_ran() {
        let mut io = InlineOutput::default();
        io.push("cmd", true);
        io.mark_entered(true, true, false);
        io.drain_frames();
        assert!(!io.is_open());
        assert!(io.take_ran(), "ran must survive a frame drain");
        assert!(
            io.take_entered().is_some(),
            "entered must survive a frame drain"
        );
    }

    #[test]
    fn take_entered_and_take_ran_clear_after_reading() {
        let mut io = InlineOutput::default();
        io.push("cmd", true);
        io.mark_entered(false, false, false);
        assert!(io.take_entered().is_some());
        assert!(io.take_ran());
        assert!(io.take_entered().is_none(), "must not report entered twice");
        assert!(!io.take_ran(), "must not report ran twice");
    }
}
