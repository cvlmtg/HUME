//! State of the `#:inline-output` terminal bracket for the command(s)
//! currently on the Steel call stack.
//!
//! Two independent facts, tracked as separate fields rather than folded into
//! one enum:
//!
//! - *A declared `#:inline-output` command is on the Steel call stack.*
//!   Nests one frame per `call!` into a declared command, pushed by
//!   [`InlineOutput::push`] and truncated back by [`InlineOutput::truncate`]
//!   (the Rust side of `%restore-inline-output!`, called with the depth
//!   `push` returned) — or with `0`, the backstop for a frame a caught Steel
//!   error left unpaired, and also how the top-level dispatch's own frame
//!   closes, since nothing calls a matching restore for it.
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
    /// Whether `Editor::run` owned the terminal when this frame was pushed.
    /// `false` means there is no alt-screen to leave for this frame (tests,
    /// headless `run_keys`); the stdout gate still opens (raw writes are
    /// safe with no TUI to protect), but [`InlineOutput::needs_enter`] never
    /// fires for it.
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
    /// `tui`. Marks `ran` when `tui` — see the module doc. Returns the depth
    /// to [`Self::truncate`] back to at the matching restore — the frame
    /// count before this push, i.e. this frame's own index.
    pub(crate) fn push(&mut self, name: &str, tui: bool) -> usize {
        let depth = self.frames.len();
        if tui {
            self.ran = true;
        }
        self.frames.push(Frame {
            name: name.to_string(),
            tui,
        });
        depth
    }

    /// Truncate the frame stack back to `depth` — the Rust side of a
    /// `call!`-armed nested command's matching `%restore-inline-output!`, or
    /// the unconditional `0` every Steel session's tail passes regardless of
    /// outcome. Drops this call's own frame and any descendant frame a
    /// caught error left unpaired above it, rather than blindly popping the
    /// top: by the time a nested `call!` returns, its own frame is not
    /// necessarily the top any more if something it called (directly or
    /// transitively) raised and the raise was caught rather than propagated
    /// — the raiser's frame is then a leak sitting above this call's own,
    /// and a blind pop would remove the leak instead of the frame that's
    /// actually closing. Never touches `entered`/`ran`: those are read at
    /// the Rust boundary right after this runs.
    pub(crate) fn truncate(&mut self, depth: usize) {
        self.frames.truncate(depth);
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

    /// Test-only seam: how many times the alt-screen has actually been left
    /// during this `Editor`'s lifetime — lets a test pin an exact count (a
    /// re-entry bug shows up as `2`, not "entered"; `0` for "never entered").
    #[cfg(test)]
    pub(crate) fn enter_count(&self) -> usize {
        self.enters
    }

    /// Test-only seam: raw frame count, for asserting exactly which frames
    /// a [`Self::truncate`] call removed.
    #[cfg(test)]
    pub(crate) fn frame_count(&self) -> usize {
        self.frames.len()
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
        let depth_inner = io.push("inner", true);
        assert!(io.is_open());
        io.truncate(depth_inner);
        assert!(io.is_open(), "outer frame must still be on the stack");
        io.truncate(0);
        assert!(!io.is_open());
    }

    /// The scenario `InlineOutput::truncate`'s own doc names: a `call!`
    /// nested two levels deep raises and the raise is caught above it,
    /// leaving its frame unpaired. The outer `call!`'s own restore must
    /// still remove both the leaked frame and its own — not just the top
    /// one a blind pop would take.
    ///
    /// Fail oracle: replace `truncate`'s body with `self.frames.pop();`
    /// (ignoring `depth`) — `frame_count()` reports `2` (only the leak was
    /// removed, `middle` itself stuck) instead of `1`.
    #[test]
    fn truncate_drops_a_leaked_descendant_frame_above_its_own() {
        let mut io = InlineOutput::default();
        io.push("outer", true);
        let depth_middle = io.push("middle", true);
        io.push("inner-leak", true); // raised and was caught; never truncated
        io.truncate(depth_middle);
        assert_eq!(
            io.frame_count(),
            1,
            "middle's own frame and the leaked descendant above it must both \
             be gone, leaving only outer"
        );
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
    fn truncate_to_zero_preserves_entered_and_ran() {
        let mut io = InlineOutput::default();
        io.push("cmd", true);
        io.mark_entered(true, true, false);
        io.truncate(0);
        assert!(!io.is_open());
        assert!(io.take_ran(), "ran must survive a full truncate");
        assert!(
            io.take_entered().is_some(),
            "entered must survive a full truncate"
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
