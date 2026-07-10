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
//! - **Captured** (`git_clone`, `git_pull_in`, `sha256_file`): returns
//!   `Output` so callers can surface stderr (or, for `sha256_file`, parse
//!   stdout) in error messages.
//! - **Inherited** (`git_clone_rev`, `git_checkout`, `curl_fetch`,
//!   `tree_sitter_build`, `unpack_zip`, `npm_install`): subprocess output
//!   flows directly to the terminal so the user sees live progress; returns
//!   `ExitStatus` only.
//! - **Piped-to-file** (`unpack_gz`): stdout is redirected to the destination
//!   file rather than the terminal or a captured buffer.
//!
//! Callers pass canonicalized paths (for sandbox `starts_with` checks), which
//! on Windows carry the `\\?\` extended-length prefix. External tools like
//! `git` and `curl` reject that prefix, so every path handed to a `Command`
//! here is normalized via `strip_unc_prefix` first (a no-op on non-Windows).

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};

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
/// Zip-slip and symlink-entry protection is delegated to the system tool
/// (modern Info-ZIP strips `../` entries; bsdtar refuses them by default) —
/// the residual risk is bounded by the sync-time sha256 pin verified before
/// unpacking, so only maintainer-vetted, hash-locked assets ever reach this
/// function.
#[cfg(unix)]
pub fn unpack_zip(src: &Path, dest_dir: &Path) -> io::Result<()> {
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
    Ok(())
}

/// See the Unix `unpack_zip` doc comment — same contract, via `tar -xf`
/// (bsdtar, built into Windows 10+) instead of `unzip`.
#[cfg(windows)]
pub fn unpack_zip(src: &Path, dest_dir: &Path) -> io::Result<()> {
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
    Ok(())
}

/// Run `npm install --ignore-scripts --prefix <dest> -- <packages…>` with
/// inherited stdio.
///
/// On Windows, npm itself is a `.cmd` shim that `CreateProcess` cannot spawn
/// directly, so the command is wrapped as `cmd /C npm …`.
pub fn npm_install(dest: &Path, packages: &[String]) -> io::Result<ExitStatus> {
    let dest = strip_unc_prefix(dest.to_path_buf());

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg("npm");
        c
    };
    #[cfg(not(windows))]
    let mut cmd = Command::new("npm");

    cmd.arg("install")
        .arg("--ignore-scripts")
        .arg("--prefix")
        .arg(&dest)
        .arg("--")
        .args(packages);
    cmd.new_process_group().status()
}

/// Whether `name` resolves to an executable file on `PATH`, without spawning
/// anything (a lookup predicate must be side-effect-free — some tools do
/// real work on `--version`, unlike the spawn-based `exe_on_path` used for
/// the Windows compiler preflight above).
///
/// Rejects `name` containing a path separator (must be a bare command name).
pub fn exe_on_search_path(name: &str) -> bool {
    if name.contains('/') || name.contains('\\') {
        return false;
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(windows)]
    let pathext_var = std::env::var_os("PATHEXT")
        .unwrap_or_else(|| std::ffi::OsString::from(".COM;.EXE;.BAT;.CMD"));
    #[cfg(not(windows))]
    let pathext_var = std::ffi::OsString::new();
    scan_path_for_exe(&path_var, &pathext_var, name)
}

/// Pure core of [`exe_on_search_path`]: scans `path_var` (a `PATH`-style,
/// platform-separator-delimited string) for `name`, resolved as an
/// executable file. `pathext_var` (Windows only; ignored on Unix) is a
/// `;`-delimited list of extensions tried in order, plus the bare name.
///
/// Takes both env values as parameters (rather than reading `std::env`
/// directly) so tests can exercise arbitrary `PATH`/`PATHEXT` combinations
/// without mutating process-global environment state.
fn scan_path_for_exe(path_var: &OsStr, pathext_var: &OsStr, name: &str) -> bool {
    #[cfg(not(windows))]
    let _ = pathext_var;
    for dir in std::env::split_paths(path_var) {
        if candidate_is_executable(&dir.join(name)) {
            return true;
        }
        #[cfg(windows)]
        for ext in pathext_var.to_string_lossy().split(';') {
            if ext.is_empty() {
                continue;
            }
            if candidate_is_executable(&dir.join(format!("{name}{ext}"))) {
                return true;
            }
        }
    }
    false
}

#[cfg(unix)]
fn candidate_is_executable(path: &PathBuf) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn candidate_is_executable(path: &PathBuf) -> bool {
    path.is_file()
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
    fn unpack_zip_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir(&src_dir).expect("mkdir src");
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
        unpack_zip(&zip_path, &dest_dir).expect("unpack_zip");

        let contents = std::fs::read(dest_dir.join("bin-name")).expect("read unpacked entry");
        assert_eq!(contents, b"binary contents");
    }

    #[test]
    #[cfg(unix)]
    fn unpack_zip_missing_source_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("missing.zip");
        let dest_dir = dir.path().join("dest");
        std::fs::create_dir(&dest_dir).expect("mkdir dest");
        assert!(unpack_zip(&src, &dest_dir).is_err());
    }

    // ── scan_path_for_exe (pure, injected PATH/PATHEXT — no env mutation) ────

    #[test]
    #[cfg(unix)]
    fn scan_path_for_exe_finds_executable_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = dir.path().join("mytool");
        std::fs::write(&exe, b"#!/bin/sh\n").expect("write");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let path_var = std::ffi::OsString::from(dir.path());
        assert!(scan_path_for_exe(&path_var, OsStr::new(""), "mytool"));
    }

    #[test]
    #[cfg(unix)]
    fn scan_path_for_exe_rejects_non_executable_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("notexec");
        std::fs::write(&file, b"data").expect("write");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        let path_var = std::ffi::OsString::from(dir.path());
        assert!(!scan_path_for_exe(&path_var, OsStr::new(""), "notexec"));
    }

    #[test]
    fn scan_path_for_exe_missing_name_returns_false() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_var = std::ffi::OsString::from(dir.path());
        assert!(!scan_path_for_exe(
            &path_var,
            OsStr::new(""),
            "nonexistent-tool"
        ));
    }

    #[test]
    #[cfg(windows)]
    fn scan_path_for_exe_resolves_pathext_suffix() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("mytool.EXE"), b"stub").expect("write");

        let path_var = std::ffi::OsString::from(dir.path());
        let pathext_var = std::ffi::OsString::from(".COM;.EXE;.BAT;.CMD");
        assert!(scan_path_for_exe(&path_var, &pathext_var, "mytool"));
    }

    #[test]
    fn exe_on_search_path_rejects_path_separators() {
        assert!(!exe_on_search_path("some/path"));
        assert!(!exe_on_search_path("some\\path"));
    }
}
