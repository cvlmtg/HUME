//! External file-change detection: stat-on-trigger, mtime+size comparison.
//!
//! Deliberately not a filesystem watcher — see `docs/ROADMAP.md`'s decision
//! table. inotify/FSEvents/kqueue/ReadDirectoryChangesW disagree on rename
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
/// already warned about" from "something changed again", and `Vanished` is
/// kept apart from `Changed` since there is no signature to recreate-and-
/// compare for a deleted file.
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
    /// The backing file no longer exists.
    Vanished,
}

/// Which trigger ran a disk check — decides whether a `Changed` state that
/// was already reported should re-fire.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiskCheckTrigger {
    /// Terminal focus, `:checktime`, return from an inline shell command. A
    /// state already reported (by an earlier ambient check, or by this same
    /// buffer having been entered before) must stay silent — nothing new to
    /// say.
    Ambient,
    /// Switching the focused pane onto this buffer (`:e`, `:b`, `:bn`,
    /// `:bp`, …). Delivers on the documented "asked about on its own next
    /// buffer-enter" promise: a change that only got a warning earlier
    /// (buffer wasn't focused yet, or `autoread` was off at the time) must
    /// still prompt now that the user has actually landed on it, even
    /// though nothing changed on disk since that warning.
    BufferEnter,
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
    /// opens a reload confirm when its `autoread` setting is on and the
    /// editor is in a mode that can show one; every other case (a
    /// non-focused buffer, `autoread` off, or a mode-blocked one) only
    /// warns. A confirm can open in `Normal`/`Extend` unconditionally, and
    /// in `Command` only while `dispatching_typed_command` is set — that is
    /// the difference between `:e`/`:b`/`:bn`/`:bp`/`:checktime` opening one
    /// as their own direct result (safe: the command line was already
    /// submitted) and an ambient check landing while the user is still
    /// typing an unsubmitted `:`/`/` line (unsafe: would steal the next
    /// keystroke and hide the in-progress line). `Insert`/`Search`/`Select`
    /// never allow one — nothing dispatches a command under those modes, so
    /// there's no legitimate case to carve out, only live typing to protect.
    ///
    /// A `Changed`/`Vanished` state already reported stays silent on a
    /// further `Ambient` check — "don't nag again for the same thing" — but
    /// a `BufferEnter` check always prompts a pending `Changed` on the
    /// focused, `autoread`-on, prompt-eligible-mode buffer regardless: that
    /// is the "asked about on its own next buffer-enter" deferred prompt
    /// the earlier warning promised, and it covers a mode-blocked report
    /// the same way it covers a non-focused one — a plain `Ambient` recheck
    /// stays silent for either until something *else* changes, only a
    /// `BufferEnter` forces the question back open. `FileMeta::signature` (the write
    /// baseline `disk_change_for` compares against) is untouched either
    /// way, so a *further* external change still reads as `Changed`.
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
                    self.report(
                        Severity::Warning,
                        format!("{name}: file no longer exists on disk"),
                    );
                }
            }
            DiskChange::Changed(sig) => {
                let buf = self.state.buffers.get_mut(bid);
                let already_reported =
                    matches!(buf.disk_state, DiskState::Changed(prev) if prev == sig);
                buf.disk_state = DiskState::Changed(sig);

                let buf = self.state.buffers.get(bid);
                let name = buf.display_name();
                let dirty = buf.is_dirty();
                let autoread = buf.overrides.autoread(&self.state.settings);
                let focused = bid == self.focused_buffer_id();

                if focused
                    && autoread
                    && self.can_open_confirm()
                    && (trigger == DiskCheckTrigger::BufferEnter || !already_reported)
                {
                    self.open_disk_change_confirm(bid, &name, dirty);
                } else if !already_reported {
                    self.report(Severity::Warning, format!("{name}: file has changed on disk"));
                }
            }
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
    /// still on screen. `docs/FUZZY-FINDERS.md` Q-B7: one modal owner at a
    /// time.
    ///
    /// Pending keys: a non-empty `pending_keys` (mid multi-key sequence, e.g.
    /// `d` waiting for its motion) or a pending `wait_char` (e.g. `f` waiting
    /// for its target char) means the very next keystroke is already spoken
    /// for — same hazard class as Insert/Command, just inside Normal mode.
    fn can_open_confirm(&self) -> bool {
        let mode_ok = match self.state.mode() {
            Mode::Normal | Mode::Extend => true,
            Mode::Command => self.state.dispatching_typed_command,
            Mode::Insert | Mode::Search | Mode::Select => false,
        };
        mode_ok
            && self.state.config.picker.is_none()
            && self.state.config.menu.is_none()
            && self.state.config.drawer.is_none()
            && self.state.pending_keys.is_empty()
            && self.state.wait_char.is_none()
    }

    /// Check every open buffer. Called from ambient trigger points (terminal
    /// focus, return from an inline shell command, `:checktime`) — never
    /// `BufferEnter`, since no single buffer among many is "the one being
    /// entered".
    pub(crate) fn check_all_disk_state(&mut self) {
        let ids: Vec<BufferId> = self.state.buffers.iter().map(|(id, _)| id).collect();
        for id in ids {
            self.check_buffer_disk_state(id, DiskCheckTrigger::Ambient);
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
    /// buffer that quietly isn't focused anymore. `try_get` still guards a
    /// non-interactive *close* of `bid` (a Steel hook, a
    /// `:reload-config`-triggered reset, both of which also drop the
    /// confirm itself) — that degrades to a silent no-op, since there is no
    /// buffer left to warn about.
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
        if let Err(e) = self.reload_from_path(bid, &path) {
            self.report(Severity::Warning, format!("{}: {e}", path.display()));
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
