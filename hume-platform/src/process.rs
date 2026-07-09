//! Process-spawning helpers.
//!
//! All `std::process::Command` usage in this workspace lives here, providing
//! a single audit surface for process spawning.
//!
//! Sandbox enforcement (path prefix checks) is the caller's responsibility;
//! these functions only perform the spawn.
//!
//! ## Captured vs inherited stdio
//!
//! - **Captured** (`git_clone`, `git_pull_in`): returns `Output` so callers can
//!   surface stderr in error messages.
//! - **Inherited** (`git_clone_rev`, `git_checkout`, `curl_fetch`,
//!   `tree_sitter_build`): subprocess output flows directly to the terminal so
//!   the user sees live progress; returns `ExitStatus` only.
//!
//! Callers pass canonicalized paths (for sandbox `starts_with` checks), which
//! on Windows carry the `\\?\` extended-length prefix. External tools like
//! `git` and `curl` reject that prefix, so every path handed to a `Command`
//! here is normalized via `strip_unc_prefix` first (a no-op on non-Windows).

use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus, Output};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

use crate::path::strip_unc_prefix;

/// Run `git clone -- <url> <dest>` and return captured output.
///
/// The caller is responsible for validating that `dest` resolves inside the
/// write sandbox before calling this.
pub fn git_clone(url: &str, dest: &Path) -> io::Result<Output> {
    let dest = strip_unc_prefix(dest.to_path_buf());
    Command::new("git")
        .args(["clone", "--", url])
        .arg(&dest)
        .output()
}

/// Run `git pull` inside `dir` and return captured output.
///
/// `dir` must already be canonicalized and sandbox-checked by the caller.
pub fn git_pull_in(dir: &Path) -> io::Result<Output> {
    let dir = strip_unc_prefix(dir.to_path_buf());
    Command::new("git").arg("pull").current_dir(&dir).output()
}

/// Clone `url` at the specific `rev` into `dest` using inherited stdio
/// (progress shown live in the terminal).
///
/// Uses `--filter=blob:none` (blobless partial clone) to avoid fetching all
/// file history.  `git_checkout` is called afterward to pin the exact revision.
pub fn git_clone_rev(url: &str, dest: &Path, rev: &str) -> io::Result<ExitStatus> {
    let dest = strip_unc_prefix(dest.to_path_buf());
    let status = Command::new("git")
        .args(["clone", "--filter=blob:none", "--", url])
        .arg(&dest)
        .new_process_group()
        .status()?;
    if !status.success() {
        return Ok(status);
    }
    git_checkout(&dest, rev)
}

/// Run `git checkout --force <rev>` inside `dir` with inherited stdio.
pub(crate) fn git_checkout(dir: &Path, rev: &str) -> io::Result<ExitStatus> {
    let dir = strip_unc_prefix(dir.to_path_buf());
    Command::new("git")
        .args(["-C"])
        .arg(&dir)
        .args(["checkout", "--force", "--end-of-options", rev, "--"])
        .new_process_group()
        .status()
}

/// Fetch `url` to `dest` via `curl -fsSL` with inherited stdio.
///
/// `dest`'s parent directory must already exist before calling this.
pub fn curl_fetch(url: &str, dest: &Path) -> io::Result<ExitStatus> {
    let dest = strip_unc_prefix(dest.to_path_buf());
    Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&dest)
        .args(["--", url])
        .new_process_group()
        .status()
}

/// Compile a tree-sitter grammar source at `src` to a shared library at `out`
/// using `tree-sitter build`, with inherited stdio.
///
/// `tree-sitter build` shells out to a C compiler via the `cc` crate. On
/// Windows that defaults to MSVC's `cl.exe`, which many machines don't have.
/// If `cl` is missing, we point `cc` at whichever alternative compiler is on
/// `PATH` (clang, gcc, or zig) via `CC`/`CXX` — see `choose_windows_compiler`.
pub fn tree_sitter_build(src: &Path, out: &Path) -> io::Result<ExitStatus> {
    let src = strip_unc_prefix(src.to_path_buf());
    let out = strip_unc_prefix(out.to_path_buf());
    let mut cmd = Command::new("tree-sitter");
    cmd.args(["build", "-o"]).arg(&out).arg(&src);

    #[cfg(windows)]
    if let Some(compiler) = choose_windows_compiler(exe_on_path) {
        let (cc, cxx) = compiler_env_vars(compiler)?;
        cmd.env("CC", cc).env("CXX", cxx);
    }

    cmd.new_process_group().status()
}

/// Whether `name` resolves to a runnable executable on `PATH`.
///
/// Spawns `name --version` and treats `NotFound` as absent; any other
/// outcome (success, nonzero exit, permission error) means the executable
/// exists. This respects `PATHEXT` on Windows without an extra dependency.
#[cfg(windows)]
fn exe_on_path(name: &str) -> bool {
    match Command::new(name).arg("--version").output() {
        Ok(_) => true,
        Err(e) => e.kind() != io::ErrorKind::NotFound,
    }
}

/// An alternate C/C++ compiler found on `PATH` to use in place of MSVC.
#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
enum WindowsCompiler {
    Clang,
    Gcc,
    Zig,
}

/// Pick an alternate compiler for `tree-sitter build` on Windows.
///
/// Returns `None` if MSVC's `cl` is available (use the platform default) or
/// if no known compiler is found. Otherwise returns the first available
/// candidate from an ordered list. `exists` is injected so this stays a
/// pure, unit-testable function independent of the real `PATH`.
#[cfg(windows)]
fn choose_windows_compiler(exists: impl Fn(&str) -> bool) -> Option<WindowsCompiler> {
    if exists("cl") {
        return None;
    }
    if exists("clang") {
        return Some(WindowsCompiler::Clang);
    }
    if exists("gcc") {
        return Some(WindowsCompiler::Gcc);
    }
    if exists("zig") {
        return Some(WindowsCompiler::Zig);
    }
    None
}

/// Resolve a `WindowsCompiler` to the `(CC, CXX)` values to set.
///
/// `gcc` is a single executable name, so it passes straight through as
/// `CC`/`CXX` — the `cc` crate never adds `--target` for GNU-family
/// compilers (cross toolchains are selected by binary name, not by flag), so
/// there's nothing to strip.
///
/// `clang` and `zig` both get `.cmd` wrappers instead of a bare executable
/// name, for two different reasons:
///
/// - **Both** need `--target` stripped (see `target_stripping_wrapper_script`)
///   — the `cc` crate detects both as clang-family and force-feeds them the
///   host's LLVM triple, which is wrong for any install not paired with
///   MSVC.
/// - **`zig` additionally** can't be named directly in `CC`, because its
///   C/C++ compilers aren't standalone executables — they're invoked as
///   `zig cc` / `zig c++`, and passing `CC="zig cc"` relies on the `cc` crate
///   splitting the value on whitespace and re-assembling wrapper + args —
///   behavior that has changed across `cc` crate versions and is broken in
///   at least one still in the wild (it drops the `cc`/`c++` argument, so
///   flags like `-O2` go straight to `zig`, which rejects them as an unknown
///   top-level command). The wrapper sidesteps this too: `CC`/`CXX` point at
///   a single filesystem token, no splitting involved.
#[cfg(windows)]
fn compiler_env_vars(compiler: WindowsCompiler) -> io::Result<(String, String)> {
    match compiler {
        WindowsCompiler::Clang => {
            let cc = write_target_stripping_wrapper("hume-clang-cc.cmd", "clang")?;
            let cxx = write_target_stripping_wrapper("hume-clang-cxx.cmd", "clang++")?;
            Ok((cc, cxx))
        }
        WindowsCompiler::Gcc => Ok(("gcc".to_string(), "g++".to_string())),
        WindowsCompiler::Zig => {
            let cc = write_target_stripping_wrapper("hume-zig-cc.cmd", "zig cc")?;
            let cxx = write_target_stripping_wrapper("hume-zig-cxx.cmd", "zig c++")?;
            Ok((cc, cxx))
        }
    }
}

/// Write a `.cmd` wrapper forwarding arguments to `invocation` (minus any
/// `--target`) into the system temp dir, and return its path.
#[cfg(windows)]
fn write_target_stripping_wrapper(file_name: &str, invocation: &str) -> io::Result<String> {
    let path = std::env::temp_dir().join(file_name);
    std::fs::write(&path, target_stripping_wrapper_script(invocation))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Batch script body forwarding arguments to `invocation` (e.g. `"clang"` or
/// `"zig cc"`), with any `--target`/`-target` (and its value) stripped.
///
/// The `cc` crate treats `clang` and `zig cc` as the same compiler family and
/// injects `--target=<host LLVM triple>` for both (e.g.
/// `x86_64-pc-windows-msvc`), on the assumption that "clang on Windows" means
/// the official LLVM.org build paired with MSVC. That assumption breaks two
/// ways:
///
/// - `zig cc` parses `--target` as a zig target query — a 3-field
///   `<arch>-<os>-<abi>` string, not the 4-field LLVM triple — so the `pc`
///   vendor component reads as an unknown OS and zig rejects it outright.
/// - A real `clang` that isn't paired with MSVC (e.g. `llvm-mingw`, MSYS2's
///   `clang64`) parses the triple fine but has no MSVC sysroot to satisfy
///   it, and forcing the msvc ABI overrides that install's own correct
///   `-gnu` default.
///
/// Dropping the flag lets each compiler fall back to its own native target,
/// which is the `-gnu` ABI whenever MSVC (`cl`) is absent — exactly the case
/// these wrappers are used in.
#[cfg(windows)]
fn target_stripping_wrapper_script(invocation: &str) -> String {
    format!(
        "@echo off\r\n\
         setlocal enabledelayedexpansion\r\n\
         set \"ARGS=\"\r\n\
         :loop\r\n\
         if \"%~1\"==\"\" goto run\r\n\
         set \"TOK=%~1\"\r\n\
         if /i \"!TOK:~0,9!\"==\"--target=\" (shift & goto loop)\r\n\
         if /i \"!TOK!\"==\"--target\" (shift & shift & goto loop)\r\n\
         if /i \"!TOK!\"==\"-target\" (shift & shift & goto loop)\r\n\
         set \"ARGS=!ARGS! %1\"\r\n\
         shift\r\n\
         goto loop\r\n\
         :run\r\n\
         {invocation} !ARGS!\r\n"
    )
}

/// Convert a non-successful `ExitStatus` to a human-readable string for error
/// messages.
pub fn exit_code_str(status: ExitStatus) -> String {
    match status.code() {
        Some(c) => format!("exit code {c}"),
        None => "killed by signal".to_string(),
    }
}

/// Whether no C compiler at all was found on `PATH` (neither `cl` nor any of
/// the `choose_windows_compiler` fallbacks). Used to append an install hint
/// to grammar-compile failure messages, without misattributing a real
/// compile error (compiler present, grammar source broken) to a missing
/// toolchain.
#[cfg(windows)]
pub fn no_windows_compiler_found() -> bool {
    // Probe `cl` once and reuse it — `choose_windows_compiler` checks `cl`
    // first internally, so passing `exe_on_path` straight through here would
    // spawn a second `cl --version` on top of the one `tree_sitter_build`
    // already ran on the same failure path.
    let has_cl = exe_on_path("cl");
    if has_cl {
        return false;
    }
    choose_windows_compiler(|name| {
        if name == "cl" {
            has_cl
        } else {
            exe_on_path(name)
        }
    })
    .is_none()
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Extension trait to set the child as its own process group leader on Unix.
///
/// On Unix: calls `setpgid(0, 0)` via `CommandExt::process_group(0)` so
/// Ctrl+C (SIGINT to the terminal's foreground process group) reaches only
/// the child, not HUME.  On other platforms this is a no-op.
trait NewProcessGroup {
    fn new_process_group(&mut self) -> &mut Self;
}

impl NewProcessGroup for Command {
    fn new_process_group(&mut self) -> &mut Self {
        #[cfg(unix)]
        self.process_group(0);
        self
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// `cl` on PATH means MSVC is usable — no override, regardless of what
    /// else is installed.
    #[test]
    #[cfg(windows)]
    fn choose_windows_compiler_prefers_msvc_when_present() {
        let choice = choose_windows_compiler(|name| matches!(name, "cl" | "clang" | "gcc" | "zig"));
        assert_eq!(choice, None, "cl present should mean no CC/CXX override");
    }

    #[test]
    #[cfg(windows)]
    fn choose_windows_compiler_falls_back_to_clang() {
        let choice = choose_windows_compiler(|name| matches!(name, "clang" | "gcc" | "zig"));
        assert_eq!(
            choice,
            Some(WindowsCompiler::Clang),
            "clang should win over gcc/zig when cl is absent"
        );
    }

    #[test]
    #[cfg(windows)]
    fn choose_windows_compiler_falls_back_to_gcc() {
        let choice = choose_windows_compiler(|name| matches!(name, "gcc" | "zig"));
        assert_eq!(
            choice,
            Some(WindowsCompiler::Gcc),
            "gcc should win over zig when cl/clang are absent"
        );
    }

    #[test]
    #[cfg(windows)]
    fn choose_windows_compiler_falls_back_to_zig() {
        let choice = choose_windows_compiler(|name| name == "zig");
        assert_eq!(
            choice,
            Some(WindowsCompiler::Zig),
            "zig should be used when no other compiler is present"
        );
    }

    #[test]
    #[cfg(windows)]
    fn choose_windows_compiler_none_when_nothing_present() {
        let choice = choose_windows_compiler(|_| false);
        assert_eq!(choice, None, "no compiler on PATH should mean no override");
    }

    /// Pin down the wrapper's exact contents: the `--target` strip (needed by
    /// both `clang` and `zig cc`, see `target_stripping_wrapper_script`) and,
    /// for `zig`, the whitespace-splitting issue that `cc`'s handling of
    /// `CC="zig cc"` used to hit (see `compiler_env_vars`).
    #[test]
    #[cfg(windows)]
    fn target_stripping_wrapper_script_forwards_args_to_invocation() {
        let expected = "@echo off\r\n\
             setlocal enabledelayedexpansion\r\n\
             set \"ARGS=\"\r\n\
             :loop\r\n\
             if \"%~1\"==\"\" goto run\r\n\
             set \"TOK=%~1\"\r\n\
             if /i \"!TOK:~0,9!\"==\"--target=\" (shift & goto loop)\r\n\
             if /i \"!TOK!\"==\"--target\" (shift & shift & goto loop)\r\n\
             if /i \"!TOK!\"==\"-target\" (shift & shift & goto loop)\r\n\
             set \"ARGS=!ARGS! %1\"\r\n\
             shift\r\n\
             goto loop\r\n\
             :run\r\n\
             zig cc !ARGS!\r\n";
        assert_eq!(target_stripping_wrapper_script("zig cc"), expected);
        assert_eq!(
            target_stripping_wrapper_script("zig c++"),
            expected.replace("zig cc !ARGS!", "zig c++ !ARGS!")
        );
        assert_eq!(
            target_stripping_wrapper_script("clang"),
            expected.replace("zig cc !ARGS!", "clang !ARGS!")
        );
        assert_eq!(
            target_stripping_wrapper_script("clang++"),
            expected.replace("zig cc !ARGS!", "clang++ !ARGS!")
        );
    }

    /// Verify that Ctrl+C (SIGINT to the child's process group) kills the
    /// child but not HUME.
    ///
    /// Behavioral guarantee: after `process_group(0)` the child is its own
    /// process group leader, so `killpg(child_pid, SIGINT)` targets only that
    /// group.  If the test process survives past the assert the guarantee holds.
    ///
    /// `nix::killpg` is used instead of spawning `kill -INT -<pgid>` because
    /// BSD `kill` and util-linux `kill` disagree on negative-pgid argument
    /// parsing — the Linux version returned exit 0 without signalling, causing
    /// `sleep` to run to completion and the test to fail.
    #[test]
    #[cfg(unix)]
    fn sigint_to_child_group_does_not_kill_hume() {
        use nix::sys::signal::{Signal, killpg};
        use nix::unistd::{Pid, setpgid};
        use std::process::Command;

        // Spawn a long-lived child so we can signal it before it exits.
        let child = Command::new("sleep")
            .arg("30")
            .new_process_group()
            .spawn()
            .expect("spawn sleep");
        let pid = Pid::from_raw(i32::try_from(child.id()).expect("pid fits i32"));

        // `process_group(0)` calls setpgid(0,0) in the child's pre-exec hook,
        // which races with the parent.  Calling setpgid(child, child) from the
        // parent is idempotent and closes the race: if the child hasn't run its
        // hook yet we set it; if it already exec'd we get EACCES (the child set
        // it first) — either way the group is correct.
        let _ = setpgid(pid, pid);

        killpg(pid, Signal::SIGINT).expect("killpg");

        // Wait for the child — must have been killed by the signal.
        let exit = child.wait_with_output().expect("wait").status;
        assert!(
            !exit.success(),
            "child should have been killed by SIGINT, got: {exit:?}"
        );

        // Reaching here means HUME survived — the guarantee holds.
    }
}
