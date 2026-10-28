//! Process-spawning helpers.
//!
//! Every `std::process::Command` that needs a process group of its own or
//! force-exit reaping goes through this module, via [`spawn_piped`] +
//! [`tracked::TrackedChild`]. Plugins run programs through the builtins this
//! module backs (`run-capture!`, `run-inline-output!`, `spawn-async!`,
//! `picker-source-spawn!`) rather than Steel's `steel/process`, which reads
//! stdout to EOF before stderr, so a child that fills its stderr pipe
//! deadlocks; [`run_capture`] drains both at once.
//!
//! Stdio is captured ([`run_capture`], `grammar_build`) or inherited so the
//! user sees live progress (`run_inline_output`).
//!
//! External tools reject Windows' `\\?\` prefix, so every path handed to a
//! `Command` here goes through `strip_unc_prefix` first.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

use crate::path::strip_unc_prefix;

/// Shared spawn/pipe machinery underneath [`line_source`] (line-batch
/// streaming, for the picker's external-command source) and [`job`]
/// (whole-output capture, for `spawn-async!` and `pipe`). Only
/// [`spawn_piped`] and its thread starters [`spawn_bridge`] and
/// [`spawn_bridge_waking`] are public, for a child that bridges its own
/// stdio, such as a language server.
pub(crate) mod child;

pub use child::{spawn_bridge, spawn_bridge_waking, spawn_piped};

/// Streaming a child's stdout into complete lines, for the picker's
/// external-command source (`picker-source-spawn!`).
pub mod line_source;

/// One-shot subprocess capture: spawns a command, feeds it its input,
/// waits for it to exit, and delivers the complete stdout/stderr/exit-status
/// once. Backs the `spawn-async!` Steel builtin and the editor's `pipe`.
pub mod job;

/// Process-wide tracking so a force-exit can still reap long-lived children.
pub mod tracked;

/// One shell command line run once per input under a shared limit on
/// running processes, for the editor's `pipe`.
pub mod batch;

/// SIGCHLD as a counter, behind [`tracked::TrackedChild::wait_for_exit`].
#[cfg(unix)]
mod exit_watch;

/// The Job Object a Windows child from [`spawn_in_own_group`] runs in.
#[cfg(windows)]
mod job_object;

/// Where a child starts: its working directory, and the `PATH` it gets, on
/// which a program named without a directory is looked up
/// ([`crate::dirs::Dirs::search_path`]; `None` starts it with `PATH` unset).
#[derive(Clone, Copy, Debug)]
pub struct Launch<'a> {
    pub cwd: &'a Path,
    pub search_path: Option<&'a OsStr>,
}

#[cfg(any(test, feature = "test-util"))]
impl<'a> Launch<'a> {
    /// A launch from `cwd` with the test process's own `PATH`, for a test
    /// that runs a real program by name.
    pub fn with_process_path(cwd: &'a Path) -> Self {
        static PROCESS_PATH: std::sync::LazyLock<Option<std::ffi::OsString>> =
            std::sync::LazyLock::new(|| std::env::var_os("PATH"));
        Self {
            cwd,
            search_path: PROCESS_PATH.as_deref(),
        }
    }
}

/// `Command::new(cmd)` with `args`, `launch`'s cwd (via `strip_unc_prefix`, a
/// no-op on non-Windows) and `PATH`, and `GIT_TERMINAL_PROMPT=0` already
/// applied: the preamble every spawn site in this crate shares. Each caller
/// layers its own remaining stdio/process-group setup on top.
///
/// Denying git a credential prompt is a blanket policy here, not a
/// git-specific carve-out, so a private repo or an expired token fails with a
/// readable git error instead of waiting for input. [`run_capture`] and
/// [`child::spawn_piped`] close the child's stdin (after any input), so a
/// prompt there would open `/dev/tty` directly and hang behind the editor.
fn base_command<I>(cmd: impl AsRef<OsStr>, args: I, launch: Launch<'_>) -> Command
where
    I: IntoIterator,
    I::Item: AsRef<OsStr>,
{
    let mut command = Command::new(cmd);
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(strip_unc_prefix(launch.cwd.to_path_buf()));
    match launch.search_path {
        Some(path) => command.env("PATH", path),
        None => command.env_remove("PATH"),
    };
    command
}

/// Run `cmd` with `args` and inherited stdio, in the caller's process group.
/// `env` entries are added to the inherited environment.
///
/// Used for `#:inline-output` Steel commands, while
/// [`crate::terminal::enter_inline_output`] has the terminal in cooked mode,
/// so a Ctrl-c reaches the child through the foreground process group.
pub fn run_inline_output(
    cmd: &str,
    args: &[String],
    launch: Launch<'_>,
    env: &[(String, String)],
) -> io::Result<ExitStatus> {
    let mut command = base_command(cmd, args, launch);
    command
        .envs(env.iter().map(|(k, v)| (k, v)))
        .status()
        .map_err(|e| spawn_failed(&command, e))
}

/// Run `cmd` with `args`, both stdout and stderr fully captured, stdin
/// closed immediately (`Stdio::null()`, so the child sees EOF on read rather
/// than racing the editor's own key reads on the terminal). Backs the
/// `run-capture!` Steel builtin.
///
/// Both pipes drain concurrently rather than one after the other (see this
/// module's doc for the deadlock that closes), and keep what a job's
/// capture keeps: stdout past `STDOUT_CAPTURE_CAP` fails the capture,
/// stderr past `STDERR_CAPTURE_CAP` is cut. The child stays in the caller's process group, so a Ctrl-c
/// during an `#:inline-output` command interrupts it as it does a
/// [`run_inline_output`] child.
pub fn run_capture(
    cmd: &str,
    args: &[String],
    launch: Launch<'_>,
) -> io::Result<std::process::Output> {
    capture(base_command(cmd, args, launch))
}

/// Compile the tree-sitter grammar at `src` to a shared library at `out`
/// by running the grammar compiler `tool` (see [`crate::dirs::Dirs::grammar_build`]),
/// its output captured and stdin closed, as [`run_capture`] runs a program:
/// the compiler writes only errors, which the caller reports. It stays in
/// the caller's process group, so a Ctrl-c during an `#:inline-output`
/// command interrupts it as it does a [`run_inline_output`] child.
pub fn grammar_build(
    tool: &Path,
    src: &Path,
    out: &Path,
    launch: Launch<'_>,
) -> io::Result<std::process::Output> {
    let tool = strip_unc_prefix(tool.to_path_buf());
    let src = strip_unc_prefix(src.to_path_buf());
    let out = strip_unc_prefix(out.to_path_buf());
    capture(base_command(
        &tool,
        [OsStr::new("-o"), out.as_os_str(), src.as_os_str()],
        launch,
    ))
}

/// `command`'s output, stdout and stderr captured and stdin closed, for
/// [`run_capture`] and [`grammar_build`]. Stderr is read on a thread of its
/// own while stdout is read here, through the capped readers a job uses. A
/// stdout past its cap fails the capture, naming the program, once the
/// child has exited.
fn capture(mut command: Command) -> io::Result<std::process::Output> {
    let mut process = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_failed(&command, e))?;
    let program = command.get_program().to_string_lossy().into_owned();
    let stdout = process.stdout.take().expect("piped stdout");
    let stderr = process.stderr.take().expect("piped stderr");
    let stderr_reader = match spawn_bridge(&program, "hume-capture-stderr", move || {
        child::read_capped(stderr, child::STDERR_CAPTURE_CAP)
    }) {
        Ok(reader) => reader,
        Err(e) => {
            let _ = process.kill();
            let _ = process.wait();
            return Err(e);
        }
    };
    let stdout = child::read_bounded(stdout, child::STDOUT_CAPTURE_CAP);
    // A panicked stderr thread degrades to empty stderr, as a job's does.
    let stderr = stderr_reader.join().unwrap_or_default();
    let status = process.wait()?;
    let stdout = stdout.map_err(|e| io::Error::new(e.kind(), format!("{program}: {e}")))?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// `e`, from any step of starting `program` (the spawn itself or a thread
/// that bridges its stdio), with a message that names the program. The
/// error's kind is kept, so a caller can still tell a missing program
/// (`NotFound`) apart.
pub fn start_failed(program: &str, e: io::Error) -> io::Error {
    io::Error::new(e.kind(), format!("cannot run '{program}': {e}"))
}

/// [`start_failed`] for `command`'s own spawn. On Unix a missing working
/// directory fails the spawn with the same error as a missing program, so
/// the message names the directory instead.
fn spawn_failed(command: &Command, e: io::Error) -> io::Error {
    let program = command.get_program().to_string_lossy();
    let e = match command.get_current_dir() {
        Some(dir) if !dir.is_dir() => io::Error::new(
            e.kind(),
            format!("directory '{}' does not exist", dir.display()),
        ),
        _ => e,
    };
    start_failed(&program, e)
}

/// The message for `program` failing: its exit, when it had one, and its
/// stderr, when it wrote any.
pub fn failure_message(program: &str, status: Option<ExitStatus>, stderr: &str) -> String {
    let mut message = format!("{program} failed");
    match status.map(|status| status.code()) {
        Some(Some(code)) => message.push_str(&format!(" (exit code {code})")),
        Some(None) => message.push_str(" (killed by signal)"),
        None => {}
    }
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        message.push_str(": ");
        message.push_str(stderr);
    }
    message
}

/// A child's output as text, each invalid UTF-8 sequence replaced with
/// U+FFFD. Valid output is reused, not copied.
pub fn decode_lossy(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// A child started in its own process group, as [`spawn_piped`] starts
/// one, with what a kill needs to reach the processes it starts: on Unix
/// the process group the child leads, on Windows the job it runs in.
/// Dropping it on Windows closes the job, which ends every process in it.
pub struct GroupedChild {
    child: std::process::Child,
    #[cfg(windows)]
    job: job_object::Job,
}

impl GroupedChild {
    /// The child's OS process id. Test-support only.
    #[cfg(test)]
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// The child itself, e.g. to `.take()` its stdio pipes.
    pub fn child_mut(&mut self) -> &mut std::process::Child {
        &mut self.child
    }
}

/// Spawns `command` so that a kill reaches every process it starts: as its
/// own process group leader on Unix, inside a new Job Object on Windows.
///
/// Pair with [`tracked::TrackedChild::new`] for a long-lived child: its
/// group lets `tracked::kill_tracked_children` reach the child's own
/// children (rust-analyzer's `proc-macro-srv`, build scripts, the command
/// under a `sh -c` or `cmd /C`, ...) instead of leaving them orphaned.
///
/// Trade-off on Unix: leaving the foreground process group costs these
/// children the kernel's SIGHUP on pty teardown (only the foreground group
/// gets one), covered instead by the LSP `processId` convention and stdin
/// EOF for everything else. Outside it, they also never get a Ctrl-c meant
/// for an inline-output child.
///
/// `process_group(0)` runs `setpgid(0, 0)` from the child's own pre-exec
/// hook, racing the parent: this function hasn't returned when that hook
/// runs, so a caller that registers/signals the returned child right away
/// could still be racing a child that hasn't exec'd. The parent-side
/// `setpgid(pid, pid)` below closes that race. It's idempotent, and `EACCES`
/// here just means the child's own call won first.
#[cfg(not(windows))]
pub(crate) fn spawn_in_own_group(command: &mut Command) -> io::Result<GroupedChild> {
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(|e| spawn_failed(command, e))?;
    #[cfg(unix)]
    {
        use nix::unistd::{Pid, setpgid};
        if let Ok(pid) = i32::try_from(child.id()) {
            let pid = Pid::from_raw(pid);
            let _ = setpgid(pid, pid);
        }
    }
    Ok(GroupedChild { child })
}

/// Spawns `command` so that a kill reaches every process it starts: inside
/// a new Job Object. See the Unix variant for the rest.
#[cfg(windows)]
pub(crate) fn spawn_in_own_group(command: &mut Command) -> io::Result<GroupedChild> {
    let (child, job) = job_object::Job::spawn(command)?;
    Ok(GroupedChild { child, job })
}

/// Kills and reaps the wrapped child if still armed when dropped.
///
/// Guards the window between spawning a long-lived child and handing it off
/// to [`tracked::TrackedChild::new`], a window `hume-lsp`'s `ServerHandle`
/// and this crate's `SpawnedLineSource` both need, since starting their
/// bridging threads is fallible and an early `?` return must not leak the
/// already-running process (`Child`'s own `Drop` does not kill). Call
/// [`into_inner`](Self::into_inner) once setup has fully succeeded to hand
/// the child off without paying the kill.
pub struct ReapOnDrop(Option<GroupedChild>);

impl ReapOnDrop {
    pub fn new(child: GroupedChild) -> Self {
        ReapOnDrop(Some(child))
    }

    /// Mutable access to the wrapped child, e.g. to `.take()` its stdio
    /// pipes before spawning bridging threads on them.
    pub fn get_mut(&mut self) -> &mut std::process::Child {
        self.0
            .as_mut()
            .expect("only emptied by into_inner, which consumes self")
            .child_mut()
    }

    /// Disarms the guard and returns the child, once every bridging thread
    /// needed to hand it off (typically to `TrackedChild::new`) has started.
    pub fn into_inner(mut self) -> GroupedChild {
        self.0
            .take()
            .expect("only emptied by into_inner, which consumes self")
    }
}

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        if let Some(mut grouped) = self.0.take() {
            let _ = grouped.child.kill();
            let _ = grouped.child.wait();
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(unix, any(test, feature = "test-util")))]
pub mod test_support;
