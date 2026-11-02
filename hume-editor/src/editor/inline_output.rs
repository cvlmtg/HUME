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
    /// `Editor::tui`'s active handle at push time. `None` means there is no
    /// alt-screen to leave (tests, headless `run_keys`): the frame still owns
    /// the terminal, but [`InlineOutput::needs_enter`] never fires for it.
    /// Captured rather than re-read because the host that later enters may
    /// be a different `Editor`/`EditorHostImpl` whose `tui` disagrees.
    tui: Option<ActiveTui>,
    /// `Editor::kitty_enabled` at push time, captured for the same reason as
    /// `tui`.
    kitty: bool,
}

/// The alt-screen was left. The screen carries the terminal state captured
/// then, so `Editor::close_inline_output_bracket` restores what it saw even
/// when a setting changed mid-command (`:set global mouse=…`).
pub(in crate::editor) enum Entered {
    Screen(InlineOutputScreen),
    /// Test-only twin of [`ActiveTui::Headless`], keeping the kitty state the
    /// frame entered under for tests to read.
    #[cfg(test)]
    Headless {
        kitty: bool,
    },
}

/// The bracket's frames, `entered` and `ran`, as the module doc describes.
#[derive(Default)]
pub(in crate::editor) struct InlineOutput {
    frames: Vec<Frame>,
    entered: Option<Entered>,
    ran: bool,
    /// Counts every [`Self::mark_entered`] in this `Editor`'s lifetime, so a
    /// test can assert the bracket fired without real terminal I/O. Never
    /// reset.
    #[cfg(test)]
    enters: usize,
}

impl InlineOutput {
    /// Push a frame for `name` with active handle `tui` and kitty state
    /// `kitty`, marking `ran` when `tui` is `Some`. Returns this frame's
    /// index, the depth to [`Self::truncate`] back to at the matching
    /// restore.
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

    /// Truncate the frame stack back to `depth`: a nested `call!`'s matching
    /// `%restore-inline-output!`, or `0` at every Steel session's tail. A
    /// raise caught below a nested `call!` leaves the raiser's frame above
    /// this call's own, so truncating, not popping, removes the right frame.
    /// Never touches `entered`/`ran`.
    pub(in crate::editor) fn truncate(&mut self, depth: usize) {
        self.frames.truncate(depth);
    }

    /// Whether a declared `#:inline-output` command is anywhere on the
    /// current call stack. It owns the terminal whenever this is true, with
    /// or without a live alt-screen to protect.
    pub(in crate::editor) fn is_open(&self) -> bool {
        !self.frames.is_empty()
    }

    /// The innermost frame's name, active handle, and kitty state if it owns
    /// the terminal and the alt-screen has not been left this session;
    /// `None` otherwise, so nesting deeper after the first print never
    /// re-enters.
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

    /// How many times the alt-screen has been left in this `Editor`'s
    /// lifetime, so a re-entry bug shows as `2`.
    #[cfg(test)]
    pub(in crate::editor) fn enter_count(&self) -> usize {
        self.enters
    }

    /// The raw frame count.
    #[cfg(test)]
    pub(in crate::editor::inline_output) fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

impl Editor {
    /// Close the `#:inline-output` bracket if it was opened, and queue the
    /// disk-change sweep a subprocess may warrant. Runs at every
    /// Steel-session boundary that can arm one: the top-level dispatch
    /// ([`Self::call_steel_command_body`]) and a hook/queued-call batch,
    /// which arms one only through a nested `(call! …)`.
    ///
    /// Every Steel session's tail already truncates the frame stack; this
    /// boundary truncates again because it owns the guarantee that no frame
    /// outlives the dispatch.
    pub(in crate::editor) fn close_inline_output_bracket(&mut self) {
        self.state.inline_output.truncate(0);
        let ran = self.state.inline_output.take_ran();
        if let Some(entered) = self.state.inline_output.take_entered() {
            // The screen holds the terminal `claim_terminal` entered, not
            // `self.tui`, so this restores the terminal it left.
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
            // The editor regained the terminal, the same trigger as
            // `TerminalEvent::FocusIn`, so it raises the same event. Its
            // reaction runs in the next `settle()`, after
            // `message_logged_this_input` is set from this dispatch;
            // `confirm_permit`'s message-shadow clause is scoped to
            // `DiskCheckTrigger::BufferEnter` so this command's own warning
            // cannot suppress the reload confirm its subprocess caused.
            self.state.queue_event(EditorEvent::OnFocusGained);
        }
    }
}

#[cfg(test)]
mod tests;
