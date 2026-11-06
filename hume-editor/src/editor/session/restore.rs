//! Rebuild a saved session onto a starting editor. The whole file is checked
//! before anything changes, so a damaged one leaves the editor as it
//! started.

use std::io::Read;
use std::path::Path;

use hume_core::{BufferId, OnUnsaved, PaneId, Severity};
use hume_engine::display_lines::DisplayLinePos;
use hume_ops::register::{Piece, is_valid_macro_register};
use hume_platform::path::AbsolutePath;
use hume_rope::line::ContentLine;
use termina::event::KeyEvent;

use super::STAMP;
use super::snapshot::{PaneSnapshot, RegisterSnapshot, SessionSnapshot, TabSnapshot};
use crate::editor::buffer::lifecycle::{self, CloseBufferRefused};
use crate::editor::commands::{open_pane_as_new_tab, open_pane_in_layout};
use crate::editor::jump_list::{JumpEntry, JumpList, kept_with_cursor};
use crate::editor::{Editor, focus, pane_state, stamped_file, tab};

/// A saved buffer as this run opened it.
#[derive(Clone, Copy)]
struct Opened {
    bid: BufferId,
    /// Whether it holds the text its saved positions were taken on.
    same_text: bool,
}

impl Editor {
    /// Restore the session saved for `session-root`, when `session-persist`
    /// is on. A session that cannot be located or read is a warning.
    /// `startup` is the scratch buffer the run started on, closed once the
    /// session shows its own buffers. Runs before the drain that announces
    /// opened buffers, so the scratch announces neither its open nor its
    /// close.
    pub(in crate::editor) fn restore_session(&mut self, startup: BufferId) {
        if !self.state.settings.session_persist {
            return;
        }
        match self.state.session_path().and_then(|path| read(&path)) {
            Ok(Some(snapshot)) => self.apply(snapshot, startup),
            Ok(None) => {}
            Err(e) => self.report(Severity::Warning, format!("session not restored: {e}")),
        }
    }

    fn apply(&mut self, snapshot: SessionSnapshot, startup: BufferId) {
        let SessionSnapshot {
            buffers,
            mru,
            tabs,
            current_tab,
            registers,
            search,
            history,
        } = snapshot;
        for (name, register) in registers {
            match register {
                RegisterSnapshot::Text(pieces) => {
                    let pieces = pieces
                        .into_iter()
                        .map(|(text, shape)| Piece::new(text, shape.into()))
                        .collect();
                    self.state.registers.write_text(name, pieces);
                }
                RegisterSnapshot::Macro(keys) => {
                    let keys = keys.into_iter().map(KeyEvent::from).collect();
                    self.state.registers.write_macro(name, keys);
                }
            }
        }
        if let Some(pattern) = search {
            self.state.registers.set_search_register(pattern);
        }
        for (kind, entries) in history {
            let ring = self.state.history.get_mut(kind);
            for entry in entries {
                ring.push(entry);
            }
        }

        let opened: Vec<Option<Opened>> = buffers
            .iter()
            .map(|saved| {
                let bid = self.open_saved(Path::new(&saved.path))?;
                let same_text = self.state.buffers.get(bid).text_md5() == saved.md5;
                Some(Opened { bid, same_text })
            })
            .collect();
        let Some(fallback) = mru.iter().rev().find_map(|&i| opened[i]) else {
            return;
        };

        let mut tab_ids = Vec::with_capacity(tabs.len());
        for (n, saved) in tabs.iter().enumerate() {
            self.rebuild_tab(saved, n == 0, &opened, fallback.bid);
            tab_ids.push(self.state.tabs.current());
        }
        tab::switch_to_tab(&mut self.state, &mut self.view, tab_ids[current_tab]);
        for &i in &mru {
            if let Some(opened) = opened[i] {
                self.state.buffers.touch_mru(opened.bid);
            }
        }
        let shown = self.view.panes[self.view.focused_pane()].buffer_id();
        self.state.buffers.touch_mru(shown);
        if self.state.buffers.try_get(startup).is_some() {
            // A scratch holding unsaved text, or one a pipe still writes
            // into, is kept.
            match lifecycle::close_buffer_and_notify(
                &mut self.view,
                &mut self.state,
                startup,
                OnUnsaved::Refuse,
            ) {
                Ok(())
                | Err(CloseBufferRefused::Unsaved(_) | CloseBufferRefused::PipeRunning(_)) => {}
            }
        }
    }

    /// Open the saved buffer at `path`, `None` when the file is gone.
    fn open_saved(&mut self, path: &Path) -> Option<BufferId> {
        let opened = AbsolutePath::from_path(path, &self.state.cwd).and_then(|typed| {
            lifecycle::open_existing_or_dedup_and_notify(&mut self.view, &mut self.state, typed)
        });
        match opened {
            Ok((bid, _)) => Some(bid),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                self.report_failed_open(path, &e);
                None
            }
        }
    }

    /// Lay `saved` out: in the active tab when `first`, which grows from its
    /// focused pane, otherwise in a new tab after the current one.
    fn rebuild_tab(
        &mut self,
        saved: &TabSnapshot,
        first: bool,
        opened: &[Option<Opened>],
        fallback: BufferId,
    ) {
        let shows = |pane: &PaneSnapshot| {
            pane.shown
                .and_then(|i| opened[i])
                .map_or(fallback, |opened| opened.bid)
        };
        if first {
            let pid = self.view.focused_pane();
            focus::switch_pane_to_buffer(
                &mut self.state,
                &mut self.view,
                pid,
                shows(&saved.panes[0]),
            );
        } else {
            open_pane_as_new_tab(&mut self.state, &mut self.view, shows(&saved.panes[0]));
        }
        let mut pids = vec![self.view.focused_pane()];
        for step in &saved.splits {
            focus::focus_pane(&mut self.state, &mut self.view, pids[step.pane]);
            let bid = shows(&saved.panes[pids.len()]);
            pids.push(open_pane_in_layout(
                &mut self.state,
                &mut self.view,
                bid,
                step.direction.into(),
            ));
        }
        focus::focus_pane(&mut self.state, &mut self.view, pids[saved.focused]);
        for (&pid, pane) in pids.iter().zip(&saved.panes) {
            self.resume_pane(pid, pane, opened);
        }
    }

    /// Put back what `pid` held for each saved buffer: its wrap pin, and its
    /// selections, scroll and jumps onto a buffer whose text is the saved
    /// one.
    fn resume_pane(&mut self, pid: PaneId, saved: &PaneSnapshot, opened: &[Option<Opened>]) {
        let state = &mut self.state;
        for kept in &saved.buffers {
            let Some(Opened { bid, same_text }) = opened[kept.buffer] else {
                continue;
            };
            if let Some(wrap) = kept.wrap {
                self.view.panes[pid].resume_wrap(bid, wrap.into());
            }
            if !same_text {
                continue;
            }
            let text = state.buffers.get(bid).text();
            if let Some(sels) = kept.selections.on(text) {
                pane_state::ensure(
                    &mut state.panes.entries,
                    &state.buffers,
                    &self.view.panes,
                    pid,
                    bid,
                )
                .set_selections(sels, text);
            }
            let (line, slot) = kept.top;
            let top = DisplayLinePos::new(ContentLine::clamped(text.rope(), line), slot);
            self.view.panes[pid].resume_scroll(bid, top);
        }
        let (jumps, cursor) = kept_with_cursor(
            saved.jumps.iter().map(|jump| {
                let Opened {
                    bid,
                    same_text: true,
                } = opened[jump.buffer]?
                else {
                    return None;
                };
                let text = state.buffers.get(bid).text();
                Some(JumpEntry::new(jump.selections.on(text)?, text, bid))
            }),
            saved.jump_cursor,
        );
        state.panes.entries[pid].jumps =
            JumpList::resumed(state.settings.jump_list_capacity, jumps, cursor);
    }
}

/// The session saved at `path`, checked. `None` when there is none or it
/// holds another format version. An error names `path`.
fn read(path: &Path) -> Result<Option<SessionSnapshot>, String> {
    STAMP.load(path, |file| {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let snapshot: SessionSnapshot = stamped_file::decode_whole(&bytes)?;
        check(&snapshot)?;
        Ok(snapshot)
    })
}

/// Whether every index in `snapshot` names something it holds.
fn check(snapshot: &SessionSnapshot) -> Result<(), &'static str> {
    let buffers = snapshot.buffers.len();
    let buffer = |i: usize| {
        (i < buffers)
            .then_some(())
            .ok_or("a buffer index past the buffers")
    };
    let mut order = snapshot.mru.clone();
    order.sort_unstable();
    if !order.into_iter().eq(0..buffers) {
        return Err("the recent-buffer order does not list each buffer once");
    }
    if snapshot.current_tab >= snapshot.tabs.len() {
        return Err("the current tab is past the tabs");
    }
    for tab in &snapshot.tabs {
        if tab.panes.len() != tab.splits.len() + 1 {
            return Err("a tab's panes do not match its splits");
        }
        if tab.splits.iter().enumerate().any(|(k, step)| step.pane > k) {
            return Err("a split names a pane made after it");
        }
        if tab.focused >= tab.panes.len() {
            return Err("a tab's focused pane is past its panes");
        }
        for pane in &tab.panes {
            pane.shown.map_or(Ok(()), buffer)?;
            pane.buffers
                .iter()
                .try_for_each(|kept| buffer(kept.buffer))?;
            pane.jumps.iter().try_for_each(|jump| buffer(jump.buffer))?;
            if pane.jump_cursor > pane.jumps.len() {
                return Err("a jump list stands past its entries");
            }
        }
    }
    if snapshot
        .registers
        .iter()
        .any(|&(name, _)| !is_valid_macro_register(name))
    {
        return Err("a register no session keeps");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
