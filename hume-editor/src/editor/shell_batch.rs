//! Running one shell command line once per input, each input on its own
//! process's stdin, within a budget of process slots the caller shares
//! across batches: the process side of `pipe`. Polled, never blocking, so a
//! caller can wait on it while it keeps reading keys.

use std::collections::VecDeque;
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

/// Where a batch stands after a [`ShellBatch::fill`].
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
    jobs: Vec<(usize, SpawnedJob)>,
    /// One slot per input, filled as its process finishes.
    outputs: Vec<Option<Captured>>,
}

impl ShellBatch {
    /// A batch running `command` under `shell` in `cwd` once per input.
    /// Nothing starts until the first [`Self::fill`].
    pub(in crate::editor) fn new(
        shell: &ShellCommand,
        command: &str,
        cwd: &Path,
        inputs: Vec<String>,
    ) -> Self {
        let (program, args) = shell.argv(command);
        Self {
            program,
            args,
            cwd: cwd.to_path_buf(),
            outputs: inputs.iter().map(|_| None).collect(),
            waiting: inputs.into_iter().enumerate().collect(),
            jobs: Vec::new(),
        }
    }

    /// How many of this batch's processes are running: the slots it holds.
    pub(in crate::editor) fn running(&self) -> usize {
        self.jobs.len()
    }

    /// Collect the processes that finished, giving their slots back to
    /// `free`. `Err` at the first process that fails.
    pub(in crate::editor) fn collect(&mut self, free: &mut usize) -> Result<(), ShellFailure> {
        let mut i = 0;
        while i < self.jobs.len() {
            match self.jobs[i].1.try_take_result() {
                Some(result) => {
                    let (index, _) = self.jobs.swap_remove(i);
                    *free += 1;
                    self.outputs[index] = Some(captured(&self.program, result)?);
                }
                None => i += 1,
            }
        }
        Ok(())
    }

    /// Start waiting inputs while `free` has slots, taking one each, and
    /// report where the batch stands. `wake` fires when a process started
    /// here finishes. `Err` when a process cannot start.
    pub(in crate::editor) fn fill(
        &mut self,
        free: &mut usize,
        wake: &WakeCallback,
    ) -> Result<BatchProgress, ShellFailure> {
        while *free > 0
            && let Some((index, input)) = self.waiting.pop_front()
        {
            let job = spawn_job(
                &self.program,
                &self.args,
                &self.cwd,
                input.into_bytes(),
                wake.clone(),
            )
            .map_err(|e| ShellFailure(format!("cannot run {}: {e}", self.program)))?;
            self.jobs.push((index, job));
            *free -= 1;
        }
        if self.jobs.is_empty() && self.waiting.is_empty() {
            let outputs = std::mem::take(&mut self.outputs)
                .into_iter()
                .map(|out| out.expect("every input ran before the batch finished"))
                .collect();
            return Ok(BatchProgress::Finished(outputs));
        }
        Ok(BatchProgress::Running {
            done: self.outputs.len() - self.waiting.len() - self.jobs.len(),
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
    if !status.is_some_and(|status| status.success()) {
        return Err(ShellFailure(hume_platform::process::failure_message(
            program, status, &stderr,
        )));
    }
    let stdout = String::from_utf8(stdout)
        .map_err(|_| ShellFailure(format!("{program} printed output that is not UTF-8")))?;
    Ok(Captured { stdout, stderr })
}
