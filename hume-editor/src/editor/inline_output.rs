//! State of the `#:inline-output` terminal bracket for the command(s)
//! currently on the Steel call stack. Three separate facts:
//!
//! - Frames: one per `call!` into a declared command, pushed by
//!   [`InlineOutput::push`] and cut back by [`InlineOutput::truncate`] (the
//!   Rust side of `%restore-inline-output!`). Truncating to `0` closes the
//!   top-level frame and any frame a caught Steel error left unpaired.
//! - `entered`: the alt-screen has been left. Happens at most once per
//!   dispatch (entering twice would pop the kitty keyboard stack twice) and is
//!   undone only by `Editor::close_inline_output_bracket`, so frame pops never
//!   touch it.
//! - `ran`: some frame owned the terminal (had a [`super::tui::ActiveTui`]).
//!   Survives frame pops like `entered`, so the post-subprocess disk-change
//!   sweep fires even for a command that never printed.

use hume_platform::terminal::InlineOutputScreen;

use super::Editor;
use super::event::EditorEvent;
use super::tui::ActiveTui;
use hume_core::Severity;

/// One `#:inline-output` command currently on the Steel call stack.
struct Frame {
    /// Printed in the running banner on first output.
    name: String,
    /// `Editor::tui`'s active handle, captured via `Tui::as_active` at push
    /// time. `None` means there is no alt-screen to leave for this frame
    /// (tests, headless `run_keys`, or an `EditorHostImpl::new` host, which
    /// has no `Tui` to capture at all and never reaches `push` in the first
    /// place; see `arm_inline_output`'s own doc). The frame still owns the
    /// terminal (a write is safe with no screen to protect), but
    /// [`InlineOutput::needs_enter`] never fires for it. Captured rather
    /// than re-read later so a caller other than the one that pushed this
    /// frame (a different `Editor`/`EditorHostImpl`'s current `tui` may
    /// disagree) still sees what was true when it was armed.
    tui: Option<ActiveTui>,
    /// `Editor::kitty_enabled` at push time, captured for the same reason as
    /// `tui`: a host completing the entry later (a different
    /// `Editor`/`EditorHostImpl`, or one with no inline-output authority at
    /// all, whose own `kitty_enabled` is meaningless) must still enter under
    /// the kitty state that was actually true when the bracket was armed.
    kitty: bool,
}

/// The alt-screen was actually left. The screen carries the terminal state
/// captured at that moment, so `Editor::close_inline_output_bracket`
/// restores what it saw rather than re-reading `Editor`/
/// `EditorSettings` fields that may have changed mid-command (`:set global
/// mouse=…` inside the very body that's running, say).
pub(in crate::editor) enum Entered {
    Screen(InlineOutputScreen),
    /// Test-only twin of [`ActiveTui::Headless`], keeping the kitty state the
    /// frame entered under for tests to read.
    #[cfg(test)]
    Headless {
        kitty: bool,
    },
}

/// See the module doc for the two facts this tracks and why they're separate
/// fields rather than one enum.
#[derive(Default)]
pub(in crate::editor) struct InlineOutput {
    frames: Vec<Frame>,
    entered: Option<Entered>,
    ran: bool,
    /// Test-only seam: counts every real [`Self::mark_entered`] this
    /// `Editor`'s lifetime has made, so a test can assert the bracket fired
    /// at least once without capturing real terminal I/O. Unlike `entered`,
    /// never reset: a session boundary clearing it would defeat the point.
    #[cfg(test)]
    enters: usize,
}

impl InlineOutput {
    /// Push a frame for `name`, currently executing with active handle
    /// `tui` (`Tui::as_active`'s result) and kitty state `kitty`. Marks `ran`
    /// when `tui` is `Some` (see the module doc). Returns the depth to
    /// [`Self::truncate`] back to at the matching restore: the frame count
    /// before this push, i.e. this frame's own index.
    pub(in crate::editor) fn push(
        &mut self,
        name: &str,
        tui: Option<ActiveTui>,
        kitty: bool,
    ) -> usize {
        let depth = self.frames.len();
        if tui.is_some() {
            self.ran = true;
        }
        self.frames.push(Frame {
            name: name.to_string(),
            tui,
            kitty,
        });
        depth
    }

    /// Truncate the frame stack back to `depth`: the Rust side of a
    /// `call!`-armed nested command's matching `%restore-inline-output!`, or
    /// the unconditional `0` every Steel session's tail passes regardless of
    /// outcome. Drops this call's own frame and any descendant frame a
    /// caught error left unpaired above it, rather than blindly popping the
    /// top: by the time a nested `call!` returns, its own frame is not
    /// necessarily the top any more if something it called (directly or
    /// transitively) raised and the raise was caught rather than propagated:
    /// the raiser's frame is then a leak sitting above this call's own,
    /// and a blind pop would remove the leak instead of the frame that's
    /// actually closing. Never touches `entered`/`ran`: those are read at
    /// the Rust boundary right after this runs.
    pub(in crate::editor) fn truncate(&mut self, depth: usize) {
        self.frames.truncate(depth);
    }

    /// Whether a declared `#:inline-output` command is anywhere on the
    /// current call stack. It owns the terminal whenever this is true, with
    /// or without a live alt-screen to protect.
    pub(in crate::editor) fn is_open(&self) -> bool {
        !self.frames.is_empty()
    }

    /// The innermost frame's name, captured active handle, and captured
    /// kitty state if it owns the terminal and the alt-screen hasn't already
    /// been left this session; `None` otherwise, including once `entered`
    /// is already set, so nesting deeper after the first real print can
    /// never re-enter.
    pub(in crate::editor) fn needs_enter(&self) -> Option<(&str, &ActiveTui, bool)> {
        if self.entered.is_some() {
            return None;
        }
        let frame = self.frames.last()?;
        Some((frame.name.as_str(), frame.tui.as_ref()?, frame.kitty))
    }

    /// Record the screen entered for this session, which
    /// [`Self::take_entered`] hands back.
    pub(in crate::editor) fn mark_entered(&mut self, entered: Entered) {
        self.entered = Some(entered);
        #[cfg(test)]
        {
            self.enters += 1;
        }
    }

    /// Take the terminal state saved by [`Self::mark_entered`], clearing it.
    /// Call once, at the Rust boundary closing the bracket.
    pub(in crate::editor) fn take_entered(&mut self) -> Option<Entered> {
        self.entered.take()
    }

    /// Take (and clear) whether any frame this session owned the terminal.
    /// Call once, at the same Rust boundary as [`Self::take_entered`].
    pub(in crate::editor) fn take_ran(&mut self) -> bool {
        std::mem::take(&mut self.ran)
    }

    /// Test-only seam: how many times the alt-screen has actually been left
    /// during this `Editor`'s lifetime. Lets a test pin an exact count (a
    /// re-entry bug shows up as `2`, not "entered"; `0` for "never entered").
    #[cfg(test)]
    pub(in crate::editor) fn enter_count(&self) -> usize {
        self.enters
    }

    /// Test-only seam: raw frame count, for asserting exactly which frames
    /// a [`Self::truncate`] call removed.
    #[cfg(test)]
    pub(in crate::editor::inline_output) fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

impl Editor {
    /// Close the `#:inline-output` bracket if a builtin actually opened it,
    /// and queue the disk-change sweep an entered/ran subprocess may
    /// warrant. Fits every Steel-session boundary that can arm one: the
    /// top-level dispatch that owns the whole bracket lifecycle
    /// ([`Self::call_steel_command_body`]), and a hook/queued-call batch
    /// that only ever arms one indirectly, through a nested `(call! …)` to
    /// an `#:inline-output` command
    /// (`hume_scripting::host::OutputHost::arm_inline_output`).
    ///
    /// By the time this runs, `InlineOutput`'s frame stack is already
    /// drained: every Steel session's own tail
    /// (`hume-scripting`'s `run_steel_session`) does that whenever
    /// its host has an output capability, as the editor's host always does,
    /// including for the top-level dispatch's own frame, which nothing else
    /// ever truncates. Truncated to zero again here regardless: this
    /// boundary owns the guarantee that no frame outlives the dispatch, and
    /// the session drain is only what satisfies it today.
    /// `entered`/`ran` survive either drain (see `InlineOutput`'s own doc);
    /// reading them here, once, is this boundary's whole job.
    pub(in crate::editor) fn close_inline_output_bracket(&mut self) {
        self.state.inline_output.truncate(0);
        let ran = self.state.inline_output.take_ran();
        if let Some(entered) = self.state.inline_output.take_entered() {
            // The screen holds the terminal `claim_terminal`
            // entered, not `self.tui`, so this always restores the terminal
            // it actually left.
            match entered {
                Entered::Screen(screen) => {
                    if let Err(e) = screen.close() {
                        self.report(
                            Severity::Error,
                            format!("inline-output restore failed: {e}"),
                        );
                    }
                }
                #[cfg(test)]
                Entered::Headless { .. } => {}
            }
            self.state.force_full_redraw = true;
        }
        if ran {
            // The editor regained the terminal: same trigger class
            // as `TerminalEvent::FocusIn`, so it raises the same event rather
            // than sweeping directly; the reaction is `OnFocusGained`'s Rust
            // handler in `Editor::react_to_event`. That reaction runs inside
            // the next `settle()`, after `message_logged_this_input` has
            // already been set from this same dispatch's own message-log
            // delta. `confirm_permit`'s message-shadow clause is scoped to
            // `DiskCheckTrigger::BufferEnter` for exactly this reason, so a
            // warning this command logged itself can't suppress the reload
            // confirm its own subprocess just caused.
            self.state.queue_event(EditorEvent::OnFocusGained);
        }
    }
}

#[cfg(test)]
mod tests;
