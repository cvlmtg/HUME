//! Process-spawning helpers.
//!
//! All `std::process::Command` usage in this workspace lives here, providing
//! a single audit surface for process spawning. General-purpose process
//! spawning for plugin code goes through Steel's own `steel/process` stdlib
//! instead (full-trust plugin model — see `docs/ROADMAP.md`'s plugin trust
//! model decision); what remains here is `run_inline_output` (process-group
//! isolation Steel can't express) plus a handful of utility functions wrapping
//! genuinely platform-conditional logic (Windows compiler selection, sha256
//! tool selection, archive unpacking with chmod) that a Scheme rewrite would
//! only make worse.
//!
//! ## Captured vs inherited stdio
//!
//! - **Captured** (`sha256_file`): returns parsed stdout.
//! - **Inherited** (`run_inline_output`, `tree_sitter_build`, `unpack_zip`):
//!   subprocess output flows directly to the terminal so the user sees live
//!   progress; returns `ExitStatus` only.
//! - **Piped-to-file** (`unpack_gz`): stdout is redirected to the destination
//!   file rather than the terminal or a captured buffer.
//!
//! On Windows, canonicalized paths carry the `\\?\` extended-length prefix.
//! External tools like `tree-sitter`, `gzip`, and `unzip`/`tar` reject that
//! prefix, so every path handed to a `Command` here is normalized via
//! `strip_unc_prefix` first (a no-op on non-Windows).

use std::fs::File;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

use crate::path::strip_unc_prefix;

/// Run `cmd` with `args`, inherited stdio, in its own process group.
///
/// Used for `#:inline-output` Steel commands — terminal raw mode is
/// temporarily disabled there (`hume_platform::terminal::enter_inline_output`
/// calls `disable_raw_mode()`), so a terminal-generated Ctrl+C (SIGINT)
/// targets the whole foreground process group. Without `process_group(0)`
/// that would kill HUME itself alongside the child. Steel's own
/// `spawn-process` has no such capability (no `setpgid`/`pre_exec` anywhere
/// in steel-core, verified against 0.8.2), so plugin code that needs this
/// safety property calls the `run-inline-output!` builtin (backed by this
/// function) instead of Steel's stdlib directly.
pub fn run_inline_output(cmd: &str, args: &[String], cwd: Option<&Path>) -> io::Result<ExitStatus> {
    let mut command = Command::new(cmd);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(strip_unc_prefix(dir.to_path_buf()));
    }
    command.new_process_group().status()
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

// ── LSP server install pipeline ──────────────────────────────────────────────
//
// sha256 verification and archive unpacking shell out to per-platform system
// tools rather than pulling in hashing/archive crates — see
// `docs/LSP-INSTALL.md`'s "Required external tools" note for the exact
// programs each platform needs.

/// Compute the sha256 digest of `path` as lowercase hex, by shelling out to
/// the platform's canonical hashing tool (`shasum -a 256` on macOS,
/// `sha256sum` on Linux, `certutil -hashfile … SHA256` on Windows).
///
/// Returns an error if the tool is missing, exits non-zero, or its output
/// can't be parsed (the raw output is quoted in that case).
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let path = strip_unc_prefix(path.to_path_buf());
    let output = sha256_command(&path).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "sha256 tool failed ({}): {}",
            exit_code_str(output.status),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    #[cfg(windows)]
    let parsed = parse_certutil_sha256_output(&stdout);
    #[cfg(not(windows))]
    let parsed = parse_unix_sha256_output(&stdout);
    parsed.ok_or_else(|| {
        io::Error::other(format!(
            "could not parse sha256 tool output: {}",
            stdout.trim()
        ))
    })
}

#[cfg(target_os = "macos")]
fn sha256_command(path: &Path) -> Command {
    let mut cmd = Command::new("shasum");
    cmd.args(["-a", "256"]).arg(path);
    cmd
}

#[cfg(all(unix, not(target_os = "macos")))]
fn sha256_command(path: &Path) -> Command {
    let mut cmd = Command::new("sha256sum");
    cmd.arg(path);
    cmd
}

#[cfg(windows)]
fn sha256_command(path: &Path) -> Command {
    let mut cmd = Command::new("certutil");
    cmd.arg("-hashfile").arg(path).arg("SHA256");
    cmd
}

/// Parse `shasum`/`sha256sum` output: the digest is the first
/// whitespace-delimited token of stdout.
///
/// Dead on Windows builds (only `parse_certutil_sha256_output` is called
/// there) but kept unconditional so both parsers are unit-testable on every
/// platform.
#[cfg_attr(windows, allow(dead_code))]
fn parse_unix_sha256_output(stdout: &str) -> Option<String> {
    let token = stdout.split_whitespace().next()?;
    if token.is_empty() {
        return None;
    }
    Some(token.to_ascii_lowercase())
}

/// Parse `certutil -hashfile … SHA256` output:
///
/// ```text
/// SHA256 hash of file <path>:
/// ab 12 cd 34 …
/// CertUtil: -hashfile command completed successfully.
/// ```
///
/// The digest is the second non-empty line, with internal whitespace
/// stripped (certutil space-separates byte pairs).
///
/// Dead on non-Windows builds (only `parse_unix_sha256_output` is called
/// there) but kept unconditional so both parsers are unit-testable on every
/// platform.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_certutil_sha256_output(stdout: &str) -> Option<String> {
    let mut lines = stdout.lines().filter(|l| !l.trim().is_empty());
    lines.next()?; // header line
    let hex_line = lines.next()?;
    let hex: String = hex_line.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.is_empty() {
        None
    } else {
        Some(hex.to_ascii_lowercase())
    }
}

/// Decode a single-file `.gz` at `src` into `dest`, by shelling out to
/// `gzip -dc` with stdout redirected to `dest`.
///
/// On Unix, `dest` is chmod'd to `0o755` after a successful decode — gzip
/// carries no file mode, and Mason's `.gz` assets are bare server
/// executables. On error, the caller is responsible for removing any partial
/// `dest` (mirroring `curl_fetch`'s cleanup contract at the Steel boundary).
pub fn unpack_gz(src: &Path, dest: &Path) -> io::Result<()> {
    let src = strip_unc_prefix(src.to_path_buf());
    let dest = strip_unc_prefix(dest.to_path_buf());
    let out_file = File::create(&dest)?;
    let status = Command::new("gzip")
        .arg("-dc")
        .arg(&src)
        .stdout(Stdio::from(out_file))
        .new_process_group()
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "gzip -dc failed ({})",
            exit_code_str(status)
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Extract the zip archive at `src` into `dest_dir`, by shelling out to
/// `unzip -o` (Unix) or `tar -xf` (Windows, via bsdtar). `dest_dir` must
/// already exist.
///
/// `bin_path` (relative to `dest_dir`) is the server binary the caller
/// expects; verified to exist after extraction, mirroring `unpack_gz`'s
/// guarantee. On Unix, every regular file in the extracted tree (not just
/// `bin_path`) is chmod'd `0o755` — unlike `.gz`, zip entries carry the
/// archive's own stored permissions and CI-built release zips routinely
/// strip the exec bit, so a layout with a wrapper script or sibling helpers
/// needs all of them executable. Every check/chmod goes through
/// `symlink_metadata` — a symlink is never followed.
///
/// Zip-slip and symlink-entry protection is delegated to the system tool
/// (modern Info-ZIP strips `../` entries; bsdtar refuses them by default) —
/// the residual risk is bounded by the sync-time sha256 pin verified before
/// unpacking, so only maintainer-vetted, hash-locked assets ever reach this
/// function.
#[cfg(unix)]
pub fn unpack_zip(src: &Path, dest_dir: &Path, bin_path: &Path) -> io::Result<()> {
    let src = strip_unc_prefix(src.to_path_buf());
    let dest_dir = strip_unc_prefix(dest_dir.to_path_buf());
    let status = Command::new("unzip")
        .arg("-o")
        .arg(&src)
        .arg("-d")
        .arg(&dest_dir)
        .new_process_group()
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "unzip failed ({})",
            exit_code_str(status)
        )));
    }
    let bin_full = dest_dir.join(bin_path);
    let missing_binary = || {
        io::Error::other(format!(
            "extracted archive is missing expected binary: {}",
            bin_path.display()
        ))
    };
    if !std::fs::symlink_metadata(&bin_full)
        .map_err(|_| missing_binary())?
        .is_file()
    {
        return Err(missing_binary());
    }
    chmod_all_regular_files(&dest_dir)?;
    Ok(())
}

/// Recursively chmod every regular file under `dir` to `0o755` —
/// `symlink_metadata` so a symlink (wherever it points, even at another
/// directory) is never followed, neither recursed into nor chmod'd.
#[cfg(unix)]
fn chmod_all_regular_files(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            chmod_all_regular_files(&path)?;
        } else if meta.is_file() {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// See the Unix `unpack_zip` doc comment — same contract, via `tar -xf`
/// (bsdtar, built into Windows 10+) instead of `unzip`. No chmod: Windows has
/// no exec-bit concept.
#[cfg(windows)]
pub fn unpack_zip(src: &Path, dest_dir: &Path, bin_path: &Path) -> io::Result<()> {
    let src = strip_unc_prefix(src.to_path_buf());
    let dest_dir = strip_unc_prefix(dest_dir.to_path_buf());
    let status = Command::new("tar")
        .arg("-xf")
        .arg(&src)
        .arg("-C")
        .arg(&dest_dir)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "tar -xf failed ({})",
            exit_code_str(status)
        )));
    }
    if !dest_dir.join(bin_path).is_file() {
        return Err(io::Error::other(format!(
            "extracted archive is missing expected binary: {}",
            bin_path.display()
        )));
    }
    Ok(())
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

    // ── run_inline_output ─────────────────────────────────────────────────────

    #[test]
    #[cfg(unix)]
    fn run_inline_output_returns_exit_status_of_child() {
        let status = run_inline_output("true", &[], None).expect("spawn true");
        assert!(status.success());

        let status = run_inline_output("false", &[], None).expect("spawn false");
        assert!(!status.success());
    }

    #[test]
    #[cfg(unix)]
    fn run_inline_output_honors_cwd() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("marker.txt"), b"hi").expect("write marker");
        let status = run_inline_output(
            "test",
            &["-f".to_string(), "marker.txt".to_string()],
            Some(dir.path()),
        )
        .expect("spawn test -f");
        assert!(status.success(), "marker.txt must be found via cwd");
    }

    #[test]
    fn run_inline_output_missing_binary_is_io_error() {
        assert!(run_inline_output("definitely-not-a-real-binary-xyz", &[], None).is_err());
    }

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

    // ── sha256 output parsing (pure, all platforms) ──────────────────────────

    #[test]
    fn parse_unix_sha256_output_takes_first_token() {
        let stdout =
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  fixture.bin\n";
        assert_eq!(
            parse_unix_sha256_output(stdout).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }

    #[test]
    fn parse_unix_sha256_output_lowercases() {
        let stdout = "ABCD1234  fixture.bin\n";
        assert_eq!(
            parse_unix_sha256_output(stdout).as_deref(),
            Some("abcd1234")
        );
    }

    #[test]
    fn parse_unix_sha256_output_empty_is_none() {
        assert_eq!(parse_unix_sha256_output(""), None);
    }

    #[test]
    fn parse_certutil_sha256_output_takes_second_line() {
        let stdout = "SHA256 hash of file fixture.bin:\r\nab 12 cd 34\r\nCertUtil: -hashfile command completed successfully.\r\n";
        assert_eq!(
            parse_certutil_sha256_output(stdout).as_deref(),
            Some("ab12cd34")
        );
    }

    #[test]
    fn parse_certutil_sha256_output_missing_hex_line_is_none() {
        assert_eq!(
            parse_certutil_sha256_output("SHA256 hash of file f:\r\n"),
            None
        );
    }

    #[test]
    fn parse_certutil_sha256_output_empty_is_none() {
        assert_eq!(parse_certutil_sha256_output(""), None);
    }

    // ── sha256_file (shells out to the real platform tool) ───────────────────

    #[test]
    fn sha256_file_matches_precomputed_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fixture.bin");
        std::fs::write(&path, b"hume").expect("write fixture");
        // `printf hume | shasum -a 256` / `sha256sum` precomputed digest.
        let expected = "604f73953b84e48e552fea0b7fed0d938b038b5b1b18f7c10f5bb640ae5e9c40";
        let got = sha256_file(&path).expect("sha256_file");
        assert_eq!(got, expected);
    }

    #[test]
    fn sha256_file_missing_source_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.bin");
        assert!(sha256_file(&path).is_err());
    }

    // ── unpack_gz ──────────────────────────────────────────────────────────────

    #[test]
    #[cfg(unix)]
    fn unpack_gz_round_trip_sets_exec_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let plain = dir.path().join("server-bin");
        std::fs::write(&plain, b"#!/bin/sh\necho hi\n").expect("write plain");

        let gz_status = Command::new("gzip")
            .arg("-k") // keep the original, we only need the .gz for the test
            .arg(&plain)
            .status()
            .expect("spawn gzip");
        assert!(gz_status.success());
        let gz_path = dir.path().join("server-bin.gz");

        let dest = dir.path().join("unpacked-bin");
        unpack_gz(&gz_path, &dest).expect("unpack_gz");

        let contents = std::fs::read(&dest).expect("read unpacked");
        assert_eq!(contents, b"#!/bin/sh\necho hi\n");
        let mode = std::fs::metadata(&dest)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "unpacked binary must be executable");
    }

    #[test]
    #[cfg(unix)]
    fn unpack_gz_missing_source_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("missing.gz");
        let dest = dir.path().join("dest");
        assert!(unpack_gz(&src, &dest).is_err());
    }

    // ── unpack_zip ─────────────────────────────────────────────────────────────

    #[test]
    #[cfg(unix)]
    fn unpack_zip_round_trip_sets_exec_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir(&src_dir).expect("mkdir src");
        // Default create mode (0o644, no exec bit) — matches what CI-built
        // release zips routinely ship, the exact case this test guards.
        std::fs::write(src_dir.join("bin-name"), b"binary contents").expect("write fixture");

        let zip_path = dir.path().join("archive.zip");
        let zip_status = Command::new("zip")
            .arg("-j") // junk paths — we only care about the entry name, not src/
            .arg(&zip_path)
            .arg(src_dir.join("bin-name"))
            .status()
            .expect("spawn zip");
        assert!(zip_status.success());

        let dest_dir = dir.path().join("dest");
        std::fs::create_dir(&dest_dir).expect("mkdir dest");
        unpack_zip(&zip_path, &dest_dir, Path::new("bin-name")).expect("unpack_zip");

        let unpacked = dest_dir.join("bin-name");
        let contents = std::fs::read(&unpacked).expect("read unpacked entry");
        assert_eq!(contents, b"binary contents");
        let mode = std::fs::metadata(&unpacked)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o755,
            "unpacked zip binary must be executable"
        );
    }

    #[test]
    #[cfg(unix)]
    fn unpack_zip_chmods_every_regular_file_not_just_bin_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir(&src_dir).expect("mkdir src");
        std::fs::write(src_dir.join("bin-name"), b"binary contents").expect("write fixture");
        // A sibling helper/wrapper the archive ships alongside the seeded
        // entry point — must end up executable too, not just "bin-name".
        std::fs::write(src_dir.join("helper"), b"helper contents").expect("write fixture");

        let zip_path = dir.path().join("archive.zip");
        let zip_status = Command::new("zip")
            .arg("-j")
            .arg(&zip_path)
            .arg(src_dir.join("bin-name"))
            .arg(src_dir.join("helper"))
            .status()
            .expect("spawn zip");
        assert!(zip_status.success());

        let dest_dir = dir.path().join("dest");
        std::fs::create_dir(&dest_dir).expect("mkdir dest");
        unpack_zip(&zip_path, &dest_dir, Path::new("bin-name")).expect("unpack_zip");

        let helper_mode = std::fs::metadata(dest_dir.join("helper"))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            helper_mode & 0o777,
            0o755,
            "a sibling file in the archive must also end up executable"
        );
    }

    #[test]
    #[cfg(unix)]
    fn unpack_zip_never_follows_a_symlink_entry_for_chmod() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let outside_target = dir.path().join("outside-target.txt");
        std::fs::write(&outside_target, b"secret").expect("write fixture");
        std::fs::set_permissions(&outside_target, std::fs::Permissions::from_mode(0o600))
            .expect("chmod fixture to non-executable");

        let src_dir = dir.path().join("src");
        std::fs::create_dir(&src_dir).expect("mkdir src");
        std::fs::write(src_dir.join("bin-name"), b"binary contents").expect("write fixture");
        std::os::unix::fs::symlink(&outside_target, src_dir.join("link-name"))
            .expect("create symlink fixture");

        let zip_path = dir.path().join("archive.zip");
        let zip_status = Command::new("zip")
            .arg("--symlinks")
            .arg("-j")
            .arg(&zip_path)
            .arg(src_dir.join("bin-name"))
            .arg(src_dir.join("link-name"))
            .status()
            .expect("spawn zip");
        assert!(zip_status.success());

        let dest_dir = dir.path().join("dest");
        std::fs::create_dir(&dest_dir).expect("mkdir dest");
        unpack_zip(&zip_path, &dest_dir, Path::new("bin-name")).expect("unpack_zip");

        assert!(
            std::fs::symlink_metadata(dest_dir.join("link-name"))
                .expect("extracted entry must exist")
                .file_type()
                .is_symlink(),
            "the archive's symlink entry must extract as a real symlink, not get dereferenced"
        );
        let target_mode = std::fs::metadata(&outside_target)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            target_mode & 0o777,
            0o600,
            "chmod must never follow the symlink to its target outside dest_dir"
        );
    }

    #[test]
    #[cfg(unix)]
    fn unpack_zip_missing_source_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("missing.zip");
        let dest_dir = dir.path().join("dest");
        std::fs::create_dir(&dest_dir).expect("mkdir dest");
        assert!(unpack_zip(&src, &dest_dir, Path::new("bin-name")).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn unpack_zip_missing_expected_binary_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir(&src_dir).expect("mkdir src");
        std::fs::write(src_dir.join("bin-name"), b"binary contents").expect("write fixture");

        let zip_path = dir.path().join("archive.zip");
        let zip_status = Command::new("zip")
            .arg("-j")
            .arg(&zip_path)
            .arg(src_dir.join("bin-name"))
            .status()
            .expect("spawn zip");
        assert!(zip_status.success());

        let dest_dir = dir.path().join("dest");
        std::fs::create_dir(&dest_dir).expect("mkdir dest");
        let err = unpack_zip(&zip_path, &dest_dir, Path::new("wrong-name")).unwrap_err();
        assert!(
            err.to_string().contains("wrong-name"),
            "expected error naming the missing binary, got: {err}"
        );
    }
}
