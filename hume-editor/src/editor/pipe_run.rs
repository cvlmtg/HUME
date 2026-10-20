//! Running a queued `pipe`. The command only queues the run; the main loop,
//! or a replay before its next key, runs it here. Until the processes
//! finish nothing else runs, no settle, no timers, no other input, so the
//! text and selections the run read are still there when its output lands.

use std::io;
use std::num::NonZeroUsize;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use hume_platform::process::WakeCallback;
use hume_scripting::PaneHandle;
use termina::EventReader;
use termina::event::{Event as TerminalEvent, KeyCode, KeyEventKind, Modifiers};

use super::commands::{self, CommandPane};
use super::replay::{DeferredChange, DeferredOutcome};
use super::shell_batch::{BatchProgress, Captured, ShellBatch, ShellFailure};
use super::{Editor, Severity};

/// What a pipe does with its command's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum PipeKind {
    /// Each selection is its process's input, and the output replaces it.
    Replace,
    /// No input; the output goes where typing goes at each cursor.
    Insert,
}

/// A `pipe` accepted by its command, waiting to run.
pub(in crate::editor) struct QueuedPipe {
    pub(in crate::editor) pane: PaneHandle,
    pub(in crate::editor) command: String,
    pub(in crate::editor) kind: PipeKind,
    /// The change the `pipe` started, kept only if the run lands its output.
    pub(in crate::editor) settles: Option<DeferredChange>,
}

/// The pipe being run, as the statusline shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PipeProgress {
    pub(crate) command: String,
    /// Processes finished.
    pub(crate) done: usize,
    /// Processes in the run, one per selection.
    pub(crate) total: usize,
}

/// What a [`PipeConsole`] saw while the run waited.
#[derive(Debug)]
pub(in crate::editor) enum WaitEvent {
    /// `Ctrl-c`: cancel the run.
    Interrupt,
    /// The terminal changed size.
    Resized,
    /// A process finished, a wake carried no input, or the timeout passed.
    Woken,
}

/// Where a pipe run reads input and draws its progress.
pub(in crate::editor) trait PipeConsole {
    /// The callback each finished process fires to end a blocked
    /// [`Self::next`].
    fn wake(&self) -> WakeCallback;
    /// Block until input arrives, [`Self::wake`] fires, or `timeout`
    /// passes, if there is one.
    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent>;
    /// Draw a frame of `ed` as it is now. Draws nothing by default.
    fn repaint(&mut self, _ed: &mut Editor) -> io::Result<()> {
        Ok(())
    }
}

/// Reads the terminal for `Ctrl-c` and resizes and draws nothing: a run
/// during replay, and the input side of the run loop's console. Every other
/// event stays queued in `reader` for the main loop to handle once the run
/// ends.
pub(in crate::editor) struct ReaderConsole {
    pub(in crate::editor) reader: EventReader,
    /// The waker `reader` blocks on.
    pub(in crate::editor) wake: WakeCallback,
}

impl PipeConsole for ReaderConsole {
    fn wake(&self) -> WakeCallback {
        Arc::clone(&self.wake)
    }

    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent> {
        let waits_for = |event: &TerminalEvent| wait_event(event).is_some();
        if !self.reader.poll(timeout, waits_for)? {
            return Ok(WaitEvent::Woken);
        }
        let event = self.reader.read(waits_for)?;
        Ok(wait_event(&event).expect("the read takes only an event the wait is for"))
    }
}

/// What `event` means to a waiting run, `None` for an event it leaves to
/// the main loop.
pub(in crate::editor) fn wait_event(event: &TerminalEvent) -> Option<WaitEvent> {
    match event {
        TerminalEvent::Key(key)
            if key.kind != KeyEventKind::Release
                && key.code == KeyCode::Char('c')
                && key.modifiers.contains(Modifiers::CONTROL) =>
        {
            Some(WaitEvent::Interrupt)
        }
        TerminalEvent::WindowResized(_) => Some(WaitEvent::Resized),
        _ => None,
    }
}

/// No terminal: a headless run waits for its processes and cannot be
/// cancelled.
pub(in crate::editor) struct Detached {
    tx: mpsc::Sender<()>,
    rx: mpsc::Receiver<()>,
}

impl Detached {
    pub(in crate::editor) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }
}

impl PipeConsole for Detached {
    fn wake(&self) -> WakeCallback {
        let tx = self.tx.clone();
        // A send fails only once this console is gone, with no one waiting.
        Arc::new(move || {
            let _ = tx.send(());
        })
    }

    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent> {
        const OPEN: &str = "the console's own sender keeps the channel open";
        match timeout {
            None => self.rx.recv().expect(OPEN),
            Some(timeout) => match self.rx.recv_timeout(timeout) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => unreachable!("{OPEN}"),
            },
        }
        Ok(WaitEvent::Woken)
    }
}

/// How a pipe run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum PipeEnd {
    /// The output replaced the selections or was inserted.
    Landed,
    /// A process failed, or the output could not land; reported.
    Failed,
    /// `Ctrl-c` or a termination request. [`Editor::run_queued_pipe_inline`]
    /// ends a run whose console failed this way too.
    Cancelled,
}

/// How long a run goes before it draws its progress, so a quick one draws
/// no frame of its own.
const PROGRESS_GRACE: Duration = Duration::from_millis(100);

/// The shortest time between two progress frames, so processes finishing
/// in quick succession draw one frame, not one each.
pub(in crate::editor) const PROGRESS_FRAME_GAP: Duration = Duration::from_millis(16);

/// How a wait ended.
enum Waited {
    Finished(Vec<Captured>),
    Failed(String),
    Cancelled,
}

impl Editor {
    /// Run the queued pipe, if any: start one process per selection, wait
    /// for them while `console` reads input, then land the output, or report
    /// the failure and change nothing. `Ctrl-c` and a termination request
    /// cancel the run and kill its processes. The change the `pipe` started
    /// is kept only when the output lands. `None` when nothing is queued.
    ///
    /// `Err` only when `console` fails; the run is cancelled first.
    pub(in crate::editor) fn run_queued_pipe(
        &mut self,
        console: &mut dyn PipeConsole,
    ) -> io::Result<Option<PipeEnd>> {
        let Some(QueuedPipe {
            pane,
            command,
            kind,
            settles,
        }) = self.state.queued_pipe.take()
        else {
            return Ok(None);
        };
        let ended = self.run_pipe(pane, command, kind, console);
        if let Some(change) = settles {
            let outcome = match ended {
                Ok(PipeEnd::Landed) => DeferredOutcome::Landed,
                Ok(PipeEnd::Failed | PipeEnd::Cancelled) | Err(_) => DeferredOutcome::Withdrawn,
            };
            self.state.settle_deferred(change, outcome);
        }
        ended.map(Some)
    }

    /// [`Self::run_queued_pipe`]'s run.
    fn run_pipe(
        &mut self,
        pane: PaneHandle,
        command: String,
        kind: PipeKind,
        console: &mut dyn PipeConsole,
    ) -> io::Result<PipeEnd> {
        let target = match CommandPane::resolve(&self.state, &self.view, pane) {
            Ok(target) => target,
            Err(e) => {
                self.report(Severity::Error, format!("pipe: {e}"));
                return Ok(PipeEnd::Failed);
            }
        };
        let inputs: Vec<String> = {
            let view = commands::pane_view(&self.state, &self.view, target);
            match kind {
                PipeKind::Replace => view.iter().map(|sel| sel.slice().to_string()).collect(),
                PipeKind::Insert => vec![String::new(); view.len()],
            }
        };
        let input_ends_line: Vec<bool> = inputs.iter().map(|input| input.ends_with('\n')).collect();
        let mut batch = ShellBatch::new(
            &self.state.settings.shell,
            &command,
            &self.state.cwd,
            inputs,
            std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN),
            console.wake(),
        );
        self.state.running_pipe = Some(PipeProgress {
            command,
            done: 0,
            total: input_ends_line.len(),
        });
        let waited = self.wait_for(&mut batch, console);
        self.state.running_pipe = None;
        Ok(match waited? {
            Waited::Finished(outputs) => commands::apply_pipe_output(
                &mut self.state,
                &self.view,
                target,
                kind,
                &input_ends_line,
                outputs,
            ),
            Waited::Failed(msg) => {
                self.report(Severity::Error, msg);
                PipeEnd::Failed
            }
            Waited::Cancelled => {
                self.report(Severity::Info, "pipe cancelled".to_string());
                PipeEnd::Cancelled
            }
        })
    }

    /// Poll `batch` to its end. A run still going after
    /// [`PROGRESS_GRACE`] draws its progress, redraws it when it changes,
    /// at most once per [`PROGRESS_FRAME_GAP`], and redraws it at once when
    /// the terminal is resized; a quicker one draws nothing.
    fn wait_for(
        &mut self,
        batch: &mut ShellBatch,
        console: &mut dyn PipeConsole,
    ) -> io::Result<Waited> {
        let mut shown = false;
        // Whether the progress differs from what the last frame showed, and
        // the earliest the next frame may draw.
        let mut stale = true;
        let mut next_frame = Instant::now() + PROGRESS_GRACE;
        loop {
            if self.state.terminate_requested() {
                return Ok(Waited::Cancelled);
            }
            match batch.poll() {
                Err(ShellFailure(msg)) => return Ok(Waited::Failed(msg)),
                Ok(BatchProgress::Finished(outputs)) => return Ok(Waited::Finished(outputs)),
                Ok(BatchProgress::Running { done }) => {
                    let progress = self
                        .state
                        .running_pipe
                        .as_mut()
                        .expect("a run shows its progress until it ends");
                    stale |= std::mem::replace(&mut progress.done, done) != done;
                    if stale && Instant::now() >= next_frame {
                        console.repaint(self)?;
                        shown = true;
                        stale = false;
                        next_frame = Instant::now() + PROGRESS_FRAME_GAP;
                    }
                }
            }
            let timeout = stale.then(|| next_frame.saturating_duration_since(Instant::now()));
            match console.next(timeout)? {
                WaitEvent::Interrupt => return Ok(Waited::Cancelled),
                WaitEvent::Resized if shown => {
                    stale = true;
                    next_frame = Instant::now();
                }
                WaitEvent::Resized | WaitEvent::Woken => {}
            }
        }
    }

    /// Run a pipe a replayed key or command queued, before the next one
    /// runs, so it sees the output. Replay draws no frames, so the run shows
    /// no progress; with a terminal attached `Ctrl-c` still cancels it.
    /// `None` when nothing is queued.
    pub(in crate::editor) fn run_queued_pipe_inline(&mut self) -> Option<PipeEnd> {
        self.state.queued_pipe.as_ref()?;
        #[cfg(test)]
        if let Some(mut console) = self.inline_pipe_console.take() {
            return self
                .run_queued_pipe(console.as_mut())
                .expect("a test console never fails");
        }
        let result = match self.tui.terminal().map(|term| term.event_reader()) {
            Some(reader) => self.run_queued_pipe(&mut ReaderConsole {
                reader,
                wake: Arc::clone(&self.state.wake),
            }),
            None => self.run_queued_pipe(&mut Detached::new()),
        };
        result.unwrap_or_else(|e| {
            self.report(
                Severity::Error,
                format!("pipe: reading the terminal failed: {e}"),
            );
            Some(PipeEnd::Cancelled)
        })
    }
}
