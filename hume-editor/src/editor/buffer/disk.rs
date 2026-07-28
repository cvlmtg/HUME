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
    /// `Vanished` always just warns — there is nothing to reload from, so
    /// never prompt. `Changed` on the *focused* buffer opens a reload
    /// confirm when its `autoread` setting is on; every other case (a
    /// non-focused buffer, or `autoread` off) only warns and flags
    /// `disk_stale`, asked about on its own next buffer-enter.
    ///
    /// A `Changed` result immediately overwrites the buffer's stored
    /// signature with the one just read, regardless of what the user does
    /// next — a later check must not re-report the exact same disk state
    /// it already reported once; only a *further* change should fire
    /// again. This is independent of `disk_stale`, which stays set (and
    /// keeps guarding `:w`) until an actual reload or write — "don't nag
    /// again for the same thing" and "don't silently overwrite" are
    /// separate questions with separate answers.
    pub(in crate::editor) fn check_buffer_disk_state(&mut self, bid: BufferId) {
        match self.disk_change_for(bid) {
            DiskChange::Unchanged => {}
            DiskChange::Vanished => {
                self.state.buffers.get_mut(bid).disk_stale = true;
                let name = self.state.buffers.get(bid).display_name();
                self.report(
                    Severity::Warning,
                    format!("{name}: file no longer exists on disk"),
                );
            }
            DiskChange::Changed(sig) => {
                let buf = self.state.buffers.get_mut(bid);
                buf.disk_stale = true;
                if let Some(meta) = buf.file_meta.as_mut() {
                    meta.set_signature(sig);
                }
                let buf = self.state.buffers.get(bid);
                let name = buf.display_name();
                let dirty = buf.is_dirty();
                let autoread = buf.overrides.autoread(&self.state.settings);
                if bid == self.focused_buffer_id() && autoread {
                    self.open_disk_change_confirm(bid, &name, dirty);
                } else {
                    self.report(Severity::Warning, format!("{name}: file has changed on disk"));
                }
            }
        }
    }

    /// Check every open buffer. Called from every trigger point (terminal
    /// focus, return from an inline shell command, `:checktime`).
    pub(crate) fn check_all_disk_state(&mut self) {
        let ids: Vec<BufferId> = self.state.buffers.iter().map(|(id, _)| id).collect();
        for id in ids {
            self.check_buffer_disk_state(id);
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
