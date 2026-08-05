//! External file-change detection: stat-on-trigger, mtime+size comparison.
//!
//! Deliberately not a filesystem watcher.
//! inotify/FSEvents/kqueue/ReadDirectoryChangesW disagree on rename
//! semantics and coalescing, and a watcher needs a thread + handle per
//! watched directory; stating on a handful of trigger points (terminal
//! focus, buffer-enter, `:checktime`) has zero background cost and behaves
//! identically on every platform. This is Neovim's own design for the same
//! feature.

use hume_engine::pipeline::BufferId;

use crate::editor::{Editor, Mode, Severity};
use crate::ui::confirm::{ConfirmChoice, ConfirmModel};

use super::Buffer;

/// Result of comparing a buffer's stored file signature against a fresh stat.
/// `Changed` carries the freshly-read signature so the caller can store it
/// back without a second stat.
pub(crate) enum DiskChange {
    /// The fresh stat genuinely matches the stored signature — the buffer is
    /// caught up with disk, whatever its prior `DiskState` was.
    Unchanged,
    Changed(hume_platform::io::FileSignature),
    Vanished,
    /// Nothing to compare (no backing file) or the stat itself failed for a
    /// reason other than `NotFound` (a momentary permission hiccup, say).
    /// Deliberately distinct from `Unchanged`: a previously reported
    /// `Changed`/`Vanished` state must survive this, since it says nothing
    /// about whether the file is actually back in sync.
    Indeterminate,
}

/// A buffer's disk state as of the last check. `InSync` and "stale" aren't
/// the only two states worth distinguishing — `Changed` also carries the
/// signature that was reported, so a later check can tell "the same change I
/// already warned about" from "something changed again", `Declined` is kept
/// apart from `Changed` so an explicit "keep" answer stays answered instead
/// of coming back on the next `BufferEnter`, and `Vanished` is kept apart
/// from both since there is no signature to recreate-and-compare for a
/// deleted file.
///
/// Deliberately never written by [`Editor::check_buffer_disk_state`] into
/// `FileMeta::signature` — that field stays the write baseline
/// (`disk_change_for`'s point of comparison) for as long as the change goes
/// un-actioned, so a *further* external change is still detected as one.
/// This enum answers a different question: "have I already reported the
/// disk state I'm looking at right now?" Reset to `InSync` whenever a fresh
/// stat genuinely matches the baseline again (a change followed by an
/// external revert) — see `check_buffer_disk_state`'s `Unchanged` arm.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DiskState {
    /// Matches what the editor last read or wrote — nothing to guard against.
    InSync,
    /// Changed externally; carries the signature that was reported.
    Changed(hume_platform::io::FileSignature),
    /// Changed externally, and the user explicitly answered the reload
    /// confirm with "keep" — carries the signature that was declined. Still
    /// stale (`stale_write_block`, not this state, is what `:w` actually
    /// re-checks, so it keeps refusing until reload or `!` regardless), and
    /// never re-prompts or re-warns for *this* signature on `Ambient`/
    /// `BufferEnter` — but a direct `:checktime` (`DiskCheckTrigger::
    /// Explicit`) still warns, since a decline silences the automatic
    /// nagging, not a check the user just asked for. A further external
    /// change (a different signature) is a fresh `Changed` and asks again on
    /// every trigger.
    Declined(hume_platform::io::FileSignature),
    /// The backing file no longer exists.
    Vanished,
}

/// Which trigger ran a disk check — decides whether a `Changed` state that
/// was already reported should re-fire.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DiskCheckTrigger {
    /// Terminal focus, return from an inline shell command. A state already
    /// reported (by an earlier ambient check, or by this same buffer having
    /// been entered before) must stay silent — nothing new to say.
    Ambient,
    /// Switching the focused pane onto this buffer (`:e`, `:b`, `:bn`,
    /// `:bp`, …). Delivers on the documented "asked about on its own next
    /// buffer-enter" promise: a change that only got a warning earlier
    /// (buffer wasn't focused yet, or `autoread` was off at the time) must
    /// still prompt now that the user has actually landed on it, even
    /// though nothing changed on disk since that warning.
    BufferEnter,
    /// `:checktime` — a direct "check now" request, not a background poll.
    /// Behaves exactly like `Ambient` (an already-reported `Changed`/
    /// `Vanished` state stays silent) except for `Declined`: a decline
    /// silences the *automatic* nagging on `Ambient`/`BufferEnter`, but a
    /// check the user just asked for on purpose must never come back
    /// silent — see the `Changed` arm's `Declined` branch in
    /// `check_buffer_disk_state`.
    Explicit,
}

impl Editor {
    /// Stat `bid`'s backing file and compare against its stored signature.
    /// Buffers with no backing file (scratch, synthetic views) always read
    /// `Indeterminate`. A stat error other than `NotFound` (a momentary
    /// permission hiccup, say) reads `Indeterminate` too — nothing to act on
    /// now, and the next trigger gets another chance.
    fn disk_change_for(&self, bid: BufferId) -> DiskChange {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return DiskChange::Indeterminate;
        };
        let Some(meta) = buf.file_meta.as_ref() else {
            return DiskChange::Indeterminate;
        };
        match hume_platform::io::read_signature(meta.resolved_path()) {
            Ok(sig) if sig == meta.signature() => DiskChange::Unchanged,
            Ok(sig) => DiskChange::Changed(sig),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DiskChange::Vanished,
            Err(_) => DiskChange::Indeterminate,
        }
    }

    /// Check one buffer's disk state and act on it.
    ///
    /// `Vanished` always just warns, once — there is nothing to reload
    /// from, so never prompt, and a state already reported must not
    /// re-warn on every later trigger. `Changed` on the *focused* buffer
    /// opens a reload confirm when its `autoread` setting is on and
    /// [`Self::can_open_confirm`] allows one; every other case (a
    /// non-focused buffer, `autoread` off, or a blocked confirm) only warns.
    ///
    /// A `Changed`/`Vanished` state already reported stays silent on a
    /// further `Ambient`/`Explicit` check — "don't nag again for the same
    /// thing" — but a `BufferEnter` check always prompts a pending `Changed`
    /// on the focused, `autoread`-on, prompt-eligible buffer regardless:
    /// that is the "asked about on its own next buffer-enter" deferred
    /// prompt the earlier warning promised. For a buffer that's
    /// prompt-eligible (focused, `autoread` on) but currently blocked from
    /// actually opening one, a `BufferEnter` still warns even if the same
    /// signature already warned once — landing on a stale buffer must never
    /// be completely silent, only a *repeat* `Ambient` recheck of the same
    /// already-reported signature stays quiet. `Declined` (the user
    /// answered "keep") never re-fires for its own signature on
    /// `Ambient`/`BufferEnter`, but does warn on `Explicit` — see
    /// `Editor::decline_disk_change` and `DiskCheckTrigger::Explicit`.
    /// `FileMeta::signature` (the write baseline `disk_change_for` compares
    /// against) is untouched by any of this, so a *further* external change
    /// still reads as fresh.
    pub(in crate::editor) fn check_buffer_disk_state(
        &mut self,
        bid: BufferId,
        trigger: DiskCheckTrigger,
    ) {
        match self.disk_change_for(bid) {
            // A genuine match resets `disk_state` — a change followed by an
            // external revert must still be detected if the file changes
            // again afterward. `Indeterminate` (pathless buffer, stat error)
            // says nothing about sync state, so it leaves `disk_state` alone.
            DiskChange::Unchanged => {
                self.state.buffers.get_mut(bid).disk_state = DiskState::InSync;
            }
            DiskChange::Indeterminate => {}
            DiskChange::Vanished => {
                let buf = self.state.buffers.get_mut(bid);
                let already_reported = buf.disk_state == DiskState::Vanished;
                buf.disk_state = DiskState::Vanished;
                if !already_reported {
                    let name = self.state.buffers.get(bid).display_name();
                    self.report_disk_state(format!("{name}: file no longer exists on disk"));
                }
            }
            DiskChange::Changed(sig) => {
                let buf = self.state.buffers.get_mut(bid);
                let declined = matches!(buf.disk_state, DiskState::Declined(prev) if prev == sig);
                if declined {
                    // The user already answered "keep" for this exact
                    // signature — leave `Declined` in place (not `Changed`)
                    // so a re-run of this same arm still recognises it as
                    // answered. `Ambient`/`BufferEnter` say nothing further;
                    // `Explicit` (`:checktime`, a direct "check now"
                    // request — see `DiskCheckTrigger::Explicit`) still
                    // warns, since a decline silences the *automatic*
                    // nagging, not a check the user just asked for on
                    // purpose.
                    if trigger == DiskCheckTrigger::Explicit {
                        let name = self.state.buffers.get(bid).display_name();
                        self.report_disk_state(format!("{name}: file has changed on disk"));
                    }
                    return;
                }
                let already_reported =
                    matches!(buf.disk_state, DiskState::Changed(prev) if prev == sig);
                buf.disk_state = DiskState::Changed(sig);

                let buf = self.state.buffers.get(bid);
                let name = buf.display_name();
                let dirty = buf.is_dirty();
                let autoread = buf.overrides.autoread(&self.state.settings);
                let focused = bid == self.focused_buffer_id();
                let promptable = focused && autoread;
                let is_buffer_enter = trigger == DiskCheckTrigger::BufferEnter;

                if promptable && self.can_open_confirm() && (is_buffer_enter || !already_reported) {
                    self.open_disk_change_confirm(bid, &name, dirty);
                } else if !already_reported || (is_buffer_enter && promptable) {
                    self.report_disk_state(format!("{name}: file has changed on disk"));
                }
            }
        }
    }

    /// Report a disk-state warning, honouring `message_logged_this_input`: if
    /// this same interactive event already logged its own warning or error
    /// (e.g. `:qa` naming the first dirty buffer, right before the focus move
    /// that landed on it triggers this check — see `can_open_confirm`'s doc),
    /// that message already owns the status line, so this lands in
    /// `:messages` only rather than displacing it. Otherwise behaves exactly
    /// like `Editor::report`. Every disk-state warning goes through this, not
    /// `report` directly, so the "never completely silent" guarantee holds
    /// without the confirm-blocked case silently overwriting an unrelated
    /// message.
    fn report_disk_state(&mut self, text: String) {
        if self.state.message_logged_this_input {
            self.state.message_log.push(Severity::Warning, text);
        } else {
            self.report(Severity::Warning, text);
        }
    }

    /// `true` if opening a confirm right now would be safe, i.e. it can't
    /// steal a keystroke from something else already mid-interaction.
    ///
    /// Mode: `Normal`/`Extend` unconditionally; `Command` only while
    /// `dispatching_typed_command` is set — that is the difference between
    /// `:e`/`:b`/`:bn`/`:bp`/`:checktime` opening one as their own direct
    /// result (safe: the command line was already submitted) and an ambient
    /// check landing while the user is still typing an unsubmitted `:`/`/`
    /// line (unsafe: would steal the next keystroke and hide the in-progress
    /// line). `Insert`/`Search`/`Select` never allow one — nothing dispatches
    /// a command under those modes, so there's no legitimate case to carve
    /// out, only live typing to protect.
    ///
    /// Overlays: the picker is mode-agnostic (opens from Normal, same as a
    /// confirm) and the menu/drawer only open from Normal/Extend too — so
    /// mode alone doesn't rule out a live overlay underneath. The confirm
    /// intercept sits above all three (`mappings/mod.rs`), so without this
    /// check an ambient trigger could silently steal every key from a picker
    /// still on screen — one modal owner at a time. A confirm already open is
    /// one of those owners itself: opening a second would replace the
    /// first's model outright, retiring an unanswered question and
    /// re-pointing the next keystroke at a different action than the one on
    /// screen when the user started reaching for it. `Editor::enter_buffer_disk_check`
    /// retires a confirm that no longer targets the buffer focus just landed
    /// on before it ever reaches this check — for *every* switch, interactive
    /// or not (a Steel/LSP `switch-to-buffer!` included, now that both run
    /// through the same `OnBufferEnter` reaction) — so this guard only needs
    /// to cover a *different* buffer's check racing a still-open, still-valid
    /// confirm.
    ///
    /// Pending keys: a non-empty `pending_keys` (mid multi-key sequence, e.g.
    /// `d` waiting for its motion) or a pending `wait_char` (e.g. `f` waiting
    /// for its target char) means the very next keystroke is already spoken
    /// for — same hazard class as Insert/Command, just inside Normal mode.
    ///
    /// Macro replay: `Editor::drain_replay_queue` calls `settle()` after
    /// every replayed key, all of it while `is_replaying` is still `true` —
    /// so any buffer-enter diff a replayed key produces is observed and
    /// warns instead of prompting, and nothing ever opens a confirm the user
    /// can no longer answer with a queued replay key. The deferred prompt
    /// still arrives on the next real buffer-enter, same as any other
    /// blocked case.
    ///
    /// Fresh message this input: `Editor::handle_input` sets
    /// `message_logged_this_input` right after dispatch whenever that input
    /// logged a new warning or error; `Editor::settle` clears it once its own
    /// drain (including the buffer-enter disk check) has run — a command
    /// that fails after moving focus (`:qa` naming the first dirty buffer)
    /// needs its own message to stay on screen, not have it replaced by an
    /// unrelated disk-change confirm. Only the confirm is blocked;
    /// `check_buffer_disk_state`'s warn fallback still runs, so this never
    /// goes fully silent.
    fn can_open_confirm(&self) -> bool {
        let mode_ok = match self.state.mode() {
            Mode::Normal | Mode::Extend => true,
            Mode::Command => self.state.dispatching_typed_command,
            Mode::Insert | Mode::Search | Mode::Select => false,
        };
        mode_ok
            && self.state.config.confirm.is_none()
            && self.state.config.picker.is_none()
            && self.state.config.menu.is_none()
            && self.state.config.drawer.is_none()
            && self.state.pending_keys.is_empty()
            && self.state.wait_char.is_none()
            && !self.state.is_replaying
            && !self.state.message_logged_this_input
    }

    /// Check every open buffer against `trigger` — `Ambient` for terminal
    /// focus and return from an inline shell command, `Explicit` for
    /// `:checktime` — never `BufferEnter`, since no single buffer among many
    /// is "the one being entered".
    pub(crate) fn check_all_disk_state(&mut self, trigger: DiskCheckTrigger) {
        debug_assert_ne!(
            trigger,
            DiskCheckTrigger::BufferEnter,
            "check_all_disk_state has no single buffer to call BufferEnter for; \
             use check_buffer_disk_state or enter_buffer_disk_check instead"
        );
        let ids: Vec<BufferId> = self.state.buffers.iter().map(|(id, _)| id).collect();
        for id in ids {
            self.check_buffer_disk_state(id, trigger);
        }
    }

    /// Run the buffer-enter disk check for `entered` — the Rust reaction to
    /// `EditorEvent::OnBufferEnter` (SPEC.md §4), called from
    /// `Editor::react_to_event` inside `settle`'s fixpoint. `OnBufferEnter`
    /// is itself a diff against `EditorState::last_entered_buffer`
    /// (`Editor::detect_buffer_enter`), so every focus-changing path —
    /// `:e`/`:b`/`:bn`/`:bp`, a picker accept, LSP goto-definition, pane
    /// close/split/cycling, a mouse click into another pane, a Steel/LSP
    /// `switch-to-buffer!` — reaches this the same way, with no per-command
    /// wiring.
    ///
    /// Also retires a confirm that no longer targets `entered`: nothing
    /// guarantees the buffer a still-open confirm targets stays focused (a
    /// mouse click has no confirm intercept at all; a handler-driven switch
    /// runs mid-`settle`). Left alone, that confirm would be unanswerable —
    /// `reload_buffer_from_disk`'s focused-buffer guard would refuse it —
    /// and would block `entered`'s own prompt via `can_open_confirm`'s
    /// `confirm.is_none()` check. Retiring it (not declining it) leaves the
    /// old buffer's `disk_state` exactly as `Changed` as it was, so the
    /// "asked about on its own next buffer-enter" promise still holds next
    /// time focus actually returns there.
    pub(in crate::editor) fn enter_buffer_disk_check(&mut self, entered: BufferId) {
        if self
            .state
            .config
            .confirm
            .as_ref()
            .is_some_and(|c| !c.targets_buffer(entered))
        {
            self.state.config.confirm = None;
        }
        self.check_buffer_disk_state(entered, DiskCheckTrigger::BufferEnter);
    }

    /// Record that the user declined to reload `bid` for the disk change
    /// currently pending on it — the confirm's `[k]eep` choice specifically
    /// (`handle_confirm_key` calls this only for that choice; `Esc` or any
    /// other key dismisses the confirm without answering it, leaving the
    /// question open for the next `BufferEnter`). Only meaningful while
    /// `disk_state` is still `Changed`; a state that moved on before the user
    /// answered (reload happened another way, the file reverted) has nothing
    /// to decline. `try_get`, not `get_mut`: same belt-and-braces as
    /// `reload_buffer_from_disk` against a non-interactive close of `bid`
    /// racing the confirm — see that method's doc.
    pub(in crate::editor) fn decline_disk_change(&mut self, bid: BufferId) {
        let Some(buf) = self.state.buffers.try_get_mut(bid) else {
            return;
        };
        if let DiskState::Changed(sig) = buf.disk_state {
            buf.disk_state = DiskState::Declined(sig);
        }
    }

    /// Open the reload confirm for `bid`. `dirty` selects the wording — a
    /// dirty buffer gets an extra note that the reload is undoable, since
    /// accepting it discards in-editor edits (recorded as one more undo
    /// step, not literally lost — see `Buffer::reload_from_text`).
    fn open_disk_change_confirm(&mut self, bid: BufferId, name: &str, dirty: bool) {
        let prompt = if dirty {
            format!("{name} has changed on disk (unsaved edits will be replaced, undo with u).")
        } else {
            format!("{name} has changed on disk.")
        };
        self.state.config.confirm = Some(ConfirmModel {
            prompt,
            choices: vec![
                ConfirmChoice {
                    key: 'r',
                    label: "reload",
                },
                ConfirmChoice {
                    key: 'k',
                    label: "keep",
                },
            ],
            action: crate::ui::confirm::ConfirmAction::ReloadBuffer(bid),
        });
    }

    /// Re-read `bid` from disk and reload it in place. Called by the
    /// confirm overlay's `[r]eload` choice.
    ///
    /// `check_buffer_disk_state` only ever opens a confirm for the focused
    /// buffer, but focus can still move before the user answers: a confirm
    /// is only checked against incoming *keys* (`handle_confirm_key`), while
    /// `prepare_frame` drains async Steel sources and pending Steel calls
    /// every frame regardless — either can call the host's
    /// `switch-to-buffer!` and move focus without ever going through key
    /// dispatch. `reload_buffer_in_place`'s focused-pane assumption
    /// (`.expect("focused pane must view the reloaded buffer")`) would panic
    /// if that happened, so this bails with a warning instead of reloading a
    /// buffer that quietly isn't focused anymore. `try_get` is belt-and-
    /// braces against a non-interactive *close* of `bid` racing the confirm:
    /// `close_buffer_and_notify` already retires a confirm that names the
    /// buffer it's closing, and `:reload-config` rebuilds `ConfigState`
    /// wholesale, so in practice `bid` never goes missing out from under an
    /// answered confirm — but if some future path ever closed a buffer
    /// without going through either, this degrades to a silent no-op instead
    /// of a panic, since there is no buffer left to warn about.
    pub(in crate::editor) fn reload_buffer_from_disk(&mut self, bid: BufferId) {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return;
        };
        if bid != self.focused_buffer_id() {
            let name = buf.display_name();
            self.report(
                Severity::Warning,
                format!("{name}: no longer focused, not reloading"),
            );
            return;
        }
        let Some(path) = buf.path().map(std::path::Path::to_path_buf) else {
            return;
        };
        let display = buf
            .display_path()
            .expect("path is Some ⇒ display_path is Some (Buffer::set_path)")
            .to_string();
        if let Err(e) = self.reload_from_path(bid, &path) {
            self.report(Severity::Warning, format!("{display}: {e}"));
        }
    }

    /// Read `path` fresh, swap it into `bid` in place (via
    /// `reload_buffer_in_place`), and report the success. Shared by the
    /// no-arg `:e`/`:e!` path (which propagates a read failure as a
    /// `CommandError`, `Severity::Error`) and `reload_buffer_from_disk`
    /// (which reports one as `Severity::Warning` and swallows it — a
    /// background reload has no caller to propagate an `Err` to). Callers
    /// choose their own error severity/propagation; this only owns the
    /// read-swap-report-success sequence, not the failure path.
    pub(in crate::editor) fn reload_from_path(
        &mut self,
        bid: BufferId,
        path: &std::path::Path,
    ) -> std::io::Result<()> {
        let doc = Buffer::from_file(path)?;
        self.reload_buffer_in_place(bid, doc);
        let name = self.state.buffers.get(bid).display_name();
        self.report(Severity::Info, format!("Reloaded {name}"));
        Ok(())
    }
}
