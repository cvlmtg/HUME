//! Per-frame drain for a session-owned spawned line source
//! (`docs/FUZZY-FINDERS.md` B5). Kept out of `picker.rs` so that module
//! stays a pure store; this is the one place `PickerItem`s get built from
//! streamed lines and the one place a spawned source's exit gets reported.

use steel::rvals::SteelVal;

use super::Editor;
use super::message_log::Severity;
use super::picker::PickerItem;

impl Editor {
    /// Arrival-driven, like the parse worker — no `AsyncSource` impl (see
    /// `async_source.rs`'s module doc); the reader thread's `WakeCallback`
    /// wakes the loop directly the moment a batch lands, so there is no
    /// deadline to poll for here.
    ///
    /// Coalesces every batch queued since the last frame into at most ONE
    /// `PickerSession::push` — `push` reranks every call, and reranking once
    /// per queued batch instead of once per frame would multiply the
    /// rerank cost by however many batches arrived this frame.
    pub(super) fn drain_picker_source(&mut self) {
        let Some(session) = self.state.config.picker.as_mut() else {
            return;
        };
        let Some(source) = session.source_mut() else {
            return;
        };
        let (lines, disconnected) = source.try_recv_batches();

        // A blank line is unmatchable noise, not a real candidate — dropped
        // here rather than in the splitter, which stays a faithful,
        // pure transcription of the byte stream (see `line_source.rs`).
        let items: Vec<PickerItem> = lines
            .into_iter()
            .filter(|line| !line.is_empty())
            .map(|line| PickerItem {
                payload: SteelVal::StringV(line.clone().into()),
                display: line,
            })
            .collect();
        if !items.is_empty() {
            let token = session.token();
            session.push(token, items);
        }

        let exit = disconnected.then(|| {
            let source = session
                .take_source()
                .expect("source_mut returned Some above, and disconnect came from the same source");
            let cmd = source.cmd().to_string();
            (cmd, source.finish())
        });
        // `session` (a borrow of `self.state.config.picker`) is not used past this
        // point, so `self.report` below can take `&mut self` freely.

        if let Some((cmd, exit)) = exit
            && let Some(status) = exit.status
            && !status.success()
        {
            self.report(
                Severity::Error,
                format!(
                    "{cmd} failed ({}): {}",
                    hume_platform::process::exit_code_str(status),
                    exit.stderr.trim()
                ),
            );
        }
    }
}
