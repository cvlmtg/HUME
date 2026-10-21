//! Running one shell command line once per input, each input on its own
//! process's stdin, within a budget of process slots the caller shares
//! across batches: the process side of the editor's `pipe`. Polled, never
//! blocking, so a caller can wait on it while it keeps reading keys.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::process::job::{JobResult, SpawnedJob, spawn_job};
use crate::wake::WakeCallback;

/// The program and leading arguments a shell command line runs under; the
/// command line itself is appended as one more argument (`sh -c <line>`).
/// The program is never empty: [`Self::from_words`], which every value goes
/// through, refuses one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    program: String,
    args: Vec<String>,
}

impl Default for ShellCommand {
    fn default() -> Self {
        Self {
            program: "sh".to_string(),
            args: vec!["-c".to_string()],
        }
    }
}

impl ShellCommand {
    /// The program to run and its arguments, `command_line` last.
    fn argv(&self, command_line: &str) -> (String, Vec<String>) {
        let mut args = self.args.clone();
        args.push(command_line.to_string());
        (self.program.clone(), args)
    }

    /// The command whose program is `words`' first and whose arguments are
    /// the rest. `Err` when there is no first word or it is empty.
    pub fn from_words(words: Vec<String>) -> Result<Self, String> {
        let mut words = words.into_iter();
        let program = words
            .next()
            .filter(|program| !program.is_empty())
            .ok_or_else(|| "needs a program to run".to_string())?;
        Ok(Self {
            program,
            args: words.collect(),
        })
    }

    /// The program followed by its arguments: what [`Self::from_words`]
    /// takes back.
    pub fn words(&self) -> Vec<String> {
        std::iter::once(&self.program)
            .chain(&self.args)
            .cloned()
            .collect()
    }
}

/// One process's output, decoded.
#[derive(Debug)]
pub struct Captured {
    pub stdout: String,
    pub stderr: String,
}

/// Where a batch stands after a [`ShellBatch::fill`].
pub enum BatchProgress {
    /// `done` processes have finished.
    Running { done: usize },
    /// Every process succeeded; one output per input, in input order.
    Finished(Vec<Captured>),
}

/// Why a batch failed, as the message to report.
#[derive(Debug)]
pub struct ShellFailure(pub String);

/// A command line run over a list of inputs. Dropping it kills every
/// process still running.
pub struct ShellBatch {
    program: String,
    args: Vec<String>,
    /// The command line in backticks, as a failure names it.
    label: String,
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
    pub fn new(shell: &ShellCommand, command: &str, cwd: &Path, inputs: Vec<String>) -> Self {
        let (program, args) = shell.argv(command);
        Self {
            program,
            args,
            label: format!("`{command}`"),
            cwd: cwd.to_path_buf(),
            outputs: inputs.iter().map(|_| None).collect(),
            waiting: inputs.into_iter().enumerate().collect(),
            jobs: Vec::new(),
        }
    }

    /// How many of this batch's processes are running: the slots it holds.
    pub fn running(&self) -> usize {
        self.jobs.len()
    }

    /// Collect the processes that finished, giving their slots back to
    /// `free`. `Err` at the first process that fails.
    pub fn collect(&mut self, free: &mut usize) -> Result<(), ShellFailure> {
        let mut i = 0;
        while i < self.jobs.len() {
            match self.jobs[i].1.try_take_result() {
                Some(result) => {
                    let (index, _) = self.jobs.swap_remove(i);
                    *free += 1;
                    self.outputs[index] = Some(captured(&self.label, result)?);
                }
                None => i += 1,
            }
        }
        Ok(())
    }

    /// Start waiting inputs while `free` has slots, taking one each, and
    /// report where the batch stands. `wake` fires when a process started
    /// here finishes. `Err` when a process cannot start.
    pub fn fill(
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
/// exit, no exit status at all, or stdout that is not UTF-8. A failure names
/// the command as `label`.
fn captured(label: &str, result: JobResult) -> Result<Captured, ShellFailure> {
    let JobResult {
        stdout,
        stderr,
        status,
    } = result;
    if !status.is_some_and(|status| status.success()) {
        return Err(ShellFailure(crate::process::failure_message(
            label, status, &stderr,
        )));
    }
    let stdout = String::from_utf8(stdout)
        .map_err(|_| ShellFailure(format!("{label} printed output that is not UTF-8")))?;
    Ok(Captured { stdout, stderr })
}

/// A `sh` command that logs `+` as it starts and `-` as it ends to `log`,
/// for a test of how many processes run at once.
#[cfg(all(unix, any(test, feature = "test-util")))]
pub fn logging_command(log: &Path) -> String {
    format!(
        "echo + >> '{log}'; sleep 0.1; echo - >> '{log}'",
        log = log.display()
    )
}

/// How many processes [`logging_command`] saw start, and the most that ran
/// at once.
#[cfg(all(unix, any(test, feature = "test-util")))]
pub fn starts_and_peak(log: &Path) -> (usize, i32) {
    let events = std::fs::read_to_string(log).expect("log written");
    let mut running = 0i32;
    let mut peak = 0;
    for line in events.lines() {
        running += if line == "+" { 1 } else { -1 };
        peak = peak.max(running);
    }
    (events.lines().filter(|&line| line == "+").count(), peak)
}

#[cfg(test)]
mod tests;
