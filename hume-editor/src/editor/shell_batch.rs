//! Running one shell command line once per input, each input on its own
//! process's stdin, at most `limit` processes at a time: the process side of
//! `pipe`. Polled, never blocking, so a caller can wait on it while it keeps
//! reading keys.

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use hume_platform::process::WakeCallback;
use hume_platform::process::job::{JobResult, SpawnedJob, spawn_job};

use super::settings::ShellCommand;

/// One process's output, decoded.
#[derive(Debug)]
pub(in crate::editor) struct Captured {
    pub(in crate::editor) stdout: String,
    pub(in crate::editor) stderr: String,
}

/// Where a batch stands after a [`ShellBatch::poll`].
pub(in crate::editor) enum BatchProgress {
    /// `done` processes have finished.
    Running { done: usize },
    /// Every process succeeded; one output per input, in input order.
    Finished(Vec<Captured>),
}

/// Why a batch failed, as the message to report.
#[derive(Debug)]
pub(in crate::editor) struct ShellFailure(pub(in crate::editor) String);

/// A command line run over a list of inputs. Dropping it kills every
/// process still running.
pub(in crate::editor) struct ShellBatch {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    /// Inputs not started yet, with their index.
    waiting: VecDeque<(usize, String)>,
    running: Vec<(usize, SpawnedJob)>,
    /// One slot per input, filled as its process finishes.
    outputs: Vec<Option<Captured>>,
    limit: NonZeroUsize,
    wake: WakeCallback,
}

impl ShellBatch {
    /// A batch running `command` under `shell` in `cwd` once per input.
    /// Nothing starts until the first [`Self::poll`]. `wake` fires whenever
    /// a process finishes.
    pub(in crate::editor) fn new(
        shell: &ShellCommand,
        command: &str,
        cwd: &Path,
        inputs: Vec<String>,
        limit: NonZeroUsize,
        wake: WakeCallback,
    ) -> Self {
        let (program, args) = shell.argv(command);
        Self {
            program,
            args,
            cwd: cwd.to_path_buf(),
            outputs: inputs.iter().map(|_| None).collect(),
            waiting: inputs.into_iter().enumerate().collect(),
            running: Vec::new(),
            limit,
            wake,
        }
    }

    /// Collect the processes that finished and start waiting inputs in the
    /// freed slots. `Err` at the first process that fails.
    pub(in crate::editor) fn poll(&mut self) -> Result<BatchProgress, ShellFailure> {
        let mut i = 0;
        while i < self.running.len() {
            match self.running[i].1.try_take_result() {
                Some(result) => {
                    let (index, _) = self.running.swap_remove(i);
                    self.outputs[index] = Some(captured(&self.program, result)?);
                }
                None => i += 1,
            }
        }
        while self.running.len() < self.limit.get()
            && let Some((index, input)) = self.waiting.pop_front()
        {
            let job = spawn_job(
                &self.program,
                &self.args,
                &self.cwd,
                input.into_bytes(),
                self.wake.clone(),
            )
            .map_err(|e| ShellFailure(format!("cannot run {}: {e}", self.program)))?;
            self.running.push((index, job));
        }
        if self.running.is_empty() && self.waiting.is_empty() {
            let outputs = std::mem::take(&mut self.outputs)
                .into_iter()
                .map(|out| out.expect("every input ran before the batch finished"))
                .collect();
            return Ok(BatchProgress::Finished(outputs));
        }
        Ok(BatchProgress::Running {
            done: self.outputs.len() - self.waiting.len() - self.running.len(),
        })
    }
}

/// A finished process's output, or why it counts as a failure: a non-zero
/// exit, no exit status at all, or stdout that is not UTF-8.
fn captured(program: &str, result: JobResult) -> Result<Captured, ShellFailure> {
    let JobResult {
        stdout,
        stderr,
        status,
    } = result;
    let detail = |what: String| {
        let stderr = stderr.trim();
        if stderr.is_empty() {
            ShellFailure(what)
        } else {
            ShellFailure(format!("{what}: {stderr}"))
        }
    };
    match status {
        Some(status) if status.success() => {}
        Some(status) => {
            return Err(detail(format!(
                "{program} failed ({})",
                hume_platform::process::exit_code_str(status)
            )));
        }
        None => return Err(detail(format!("{program} failed"))),
    }
    let stdout = String::from_utf8(stdout)
        .map_err(|_| ShellFailure(format!("{program} printed output that is not UTF-8")))?;
    Ok(Captured { stdout, stderr })
}
