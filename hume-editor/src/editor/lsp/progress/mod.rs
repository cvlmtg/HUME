//! `$/progress` tracking: one [`ProgressTask`] per active work-done-progress
//! token, and the [`SpinnerClock`] that paces the statusline's loading
//! animation while any server is starting up or reporting progress.

use std::time::{Duration, Instant};

use hume_lsp::backend::ServerId;

use crate::editor::{Editor, Severity};

/// How often the loading spinner advances a frame — independent of how
/// often `drain_lsp` itself runs (`next_wake` may wake faster than this
/// while a handshake or `$/progress` task is active).
pub(super) const SPINNER_INTERVAL: Duration = Duration::from_millis(100);

/// Monotonic animation-frame counter for the statusline spinner
/// (`elements/diagnostics.rs`'s loading state). `frame` is a plain `usize`
/// so the render side (`format`) stays a deterministic, clock-free function
/// of its inputs — only this clock needs a real `Instant`.
#[derive(Default)]
pub(super) struct SpinnerClock {
    pub(super) frame: usize,
    last_advance: Option<Instant>,
}

impl SpinnerClock {
    /// Bumps `frame` by one if at least `SPINNER_INTERVAL` has elapsed since
    /// the last advance (or this is the first call).
    pub(super) fn maybe_advance(&mut self, now: Instant) {
        if self
            .last_advance
            .is_none_or(|last| now.saturating_duration_since(last) >= SPINNER_INTERVAL)
        {
            self.frame = self.frame.wrapping_add(1);
            self.last_advance = Some(now);
        }
    }
}

/// One active work-done-progress task, built from a `begin` notification and
/// updated in place by `report`s. `percentage` is optional per the LSP spec —
/// a `report` omitting it leaves it unchanged, so it's merged rather than the
/// task being replaced wholesale.
#[derive(Debug, Clone)]
pub(super) struct ProgressTask {
    // Not read in production — the statusline only shows the spinner +
    // percentage (`introspect::LspActivity::Progress` carries no title).
    // Kept so the `$/progress` begin/report merge machine has something to
    // assert against in tests, via `LspState::progress_title_for_test`.
    #[allow(dead_code)]
    pub(crate) title: String,
    pub(crate) percentage: Option<u32>,
}

impl Editor {
    /// Typed handling of `$/progress`: begin/end logged at Trace; the task
    /// itself is tracked on `ServerEntry.progress` for the statusline
    /// spinner, with `report`s merged into it (absent fields mean
    /// "unchanged" per the LSP spec).
    pub(super) fn handle_progress(
        &mut self,
        server_id: ServerId,
        params: lsp_types::ProgressParams,
    ) {
        let name = self.lsp_server_name(server_id);
        let token = match params.token {
            lsp_types::NumberOrString::Number(n) => n.to_string(),
            lsp_types::NumberOrString::String(s) => s,
        };
        // `ProgressParamsValue` has exactly one variant — irrefutable.
        let lsp_types::ProgressParamsValue::WorkDone(progress) = params.value;
        match progress {
            lsp_types::WorkDoneProgress::Begin(begin) => {
                self.report(Severity::Trace, format!("{name}: {} started", begin.title));
                if let Some(entry) = self.lsp.servers.get_mut(&server_id) {
                    entry.progress.push((
                        token,
                        ProgressTask {
                            title: begin.title,
                            percentage: begin.percentage,
                        },
                    ));
                }
            }
            lsp_types::WorkDoneProgress::Report(report) => {
                let Some(entry) = self.lsp.servers.get_mut(&server_id) else {
                    return;
                };
                let Some((_, task)) = entry.progress.iter_mut().find(|(t, _)| *t == token) else {
                    return; // report for an unknown token — nothing to merge into
                };
                // An absent percentage means "unchanged" per the LSP spec — merge, don't overwrite.
                if let Some(percentage) = report.percentage {
                    task.percentage = Some(percentage);
                }
            }
            lsp_types::WorkDoneProgress::End(_) => {
                self.report(Severity::Trace, format!("{name}: progress finished"));
                if let Some(entry) = self.lsp.servers.get_mut(&server_id) {
                    entry.progress.retain(|(t, _)| *t != token);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
