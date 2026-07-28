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

use crate::editor::{Editor, Severity};
use crate::ui::confirm::{ConfirmChoice, ConfirmModel};

use super::Buffer;

/// Result of comparing a buffer's stored file signature against a fresh stat.
/// `Changed` carries the freshly-read signature so the caller can store it
/// back without a second stat.
pub(crate) enum DiskChange {
    Unchanged,
    Changed(hume_platform::io::FileSignature),
    Vanished,
}

/// A buffer's disk state as of the last check, replacing a plain
/// `disk_stale: bool`. `InSync` and "stale" aren't the only two states worth
/// distinguishing — `Changed` also carries the signature that was reported,
/// so a later check can tell "the same change I already warned about" from
/// "something changed again", and `Vanished` is kept apart from `Changed`
/// since there is no signature to recreate-and-compare for a deleted file.
///
/// Deliberately never written by [`Editor::check_buffer_disk_state`] into
/// `FileMeta::signature` — that field stays the write baseline
/// (`disk_change_for`'s point of comparison) for as long as the change goes
/// un-actioned, so a *further* external change is still detected as one.
/// This enum answers a different question: "have I already reported the
/// disk state I'm looking at right now?"
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
    /// `Unchanged`. A stat error other than `NotFound` (a momentary
    /// permission hiccup, say) is also treated as `Unchanged` — nothing to
    /// act on now, and the next trigger gets another chance.
    fn disk_change_for(&self, bid: BufferId) -> DiskChange {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return DiskChange::Unchanged;
        };
        let Some(meta) = buf.file_meta.as_ref() else {
            return DiskChange::Unchanged;
        };
        match hume_platform::io::read_signature(meta.resolved_path()) {
            Ok(sig) if sig == meta.signature() => DiskChange::Unchanged,
            Ok(sig) => DiskChange::Changed(sig),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DiskChange::Vanished,
            Err(_) => DiskChange::Unchanged,
        }
    }

    /// Check one buffer's disk state and act on it.
    ///
    /// `Vanished` always just warns, once — there is nothing to reload
    /// from, so never prompt, and a state already reported must not
    /// re-warn on every later trigger. `Changed` on the *focused* buffer
    /// opens a reload confirm when its `autoread` setting is on; every
    /// other case (a non-focused buffer, or `autoread` off) only warns.
    ///
    /// A `Changed`/`Vanished` state already reported stays silent on a
    /// further `Ambient` check — "don't nag again for the same thing" — but
    /// a `BufferEnter` check always prompts a pending `Changed` on the
    /// focused, `autoread`-on buffer regardless: that is the "asked about
    /// on its own next buffer-enter" deferred prompt the non-focused/
    /// `autoread`-off warning promised earlier. `FileMeta::signature` (the
    /// write baseline `disk_change_for` compares against) is untouched
    /// either way, so a *further* external change still reads as `Changed`.
    pub(in crate::editor) fn check_buffer_disk_state(
        &mut self,
        bid: BufferId,
        trigger: DiskCheckTrigger,
    ) {
        match self.disk_change_for(bid) {
            DiskChange::Unchanged => {}
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

                if focused && autoread && (trigger == DiskCheckTrigger::BufferEnter || !already_reported)
                {
                    self.open_disk_change_confirm(bid, &name, dirty);
                } else if !already_reported {
                    self.report(Severity::Warning, format!("{name}: file has changed on disk"));
                }
            }
        }
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
    /// Only ever invoked while `bid` is still the focused buffer —
    /// `check_buffer_disk_state` only opens a confirm for the focused
    /// buffer, the confirm intercept consumes every key unconditionally
    /// while open, and no interactive command can run in between — so
    /// `reload_buffer_in_place`'s focused-pane assumption always holds on
    /// the interactive path. `try_get` still guards this: a non-interactive
    /// close of `bid` (a Steel hook, a `:reload-config`-triggered reset,
    /// both of which also drop the confirm itself) degrades to a silent
    /// no-op rather than a panic.
    pub(in crate::editor) fn reload_buffer_from_disk(&mut self, bid: BufferId) {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return;
        };
        let Some(path) = buf.path().map(std::path::Path::to_path_buf) else {
            return;
        };
        let name = buf.display_name();
        match Buffer::from_file(&path) {
            Ok(doc) => {
                self.reload_buffer_in_place(bid, doc);
                self.report(Severity::Info, format!("Reloaded {name}"));
            }
            Err(e) => self.report(Severity::Warning, format!("{}: {e}", path.display())),
        }
    }
}
