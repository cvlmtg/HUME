//! A compiler for Windows machines without MSVC.
//!
//! `cc` assumes MSVC on an `-msvc` target and names `cl.exe` even when it
//! is not installed. It also treats `clang` and `zig cc` as clang-family and
//! passes them `--target=<host LLVM triple>`, which `zig cc` rejects and
//! which forces the MSVC ABI onto a clang paired with MinGW. A fallback
//! driver here runs with no `cc` arguments at all, so each compiler keeps
//! its own native target.

use std::io;
use std::process::Command;

use super::{Compiler, Flavor, TARGET};
use crate::Error;

/// A compiler found on `PATH`, in the order they are preferred.
#[derive(Debug, PartialEq, Eq)]
enum WindowsCompiler {
    Msvc,
    Clang,
    Gcc,
    Zig,
}

/// The first compiler `exists` reports, `cl` standing for MSVC.
/// `exists` is injected so the order is testable without a real `PATH`.
fn choose_windows_compiler(exists: impl Fn(&str) -> bool) -> Option<WindowsCompiler> {
    [
        ("cl", WindowsCompiler::Msvc),
        ("clang", WindowsCompiler::Clang),
        ("gcc", WindowsCompiler::Gcc),
        ("zig", WindowsCompiler::Zig),
    ]
    .into_iter()
    .find(|(name, _)| exists(name))
    .map(|(_, compiler)| compiler)
}

/// A non-MSVC driver, or `None` to let `cc` choose: when `CC`/`CXX` names
/// one, or MSVC is installed.
pub(super) fn without_msvc(cpp: bool) -> Result<Option<Compiler>, Error> {
    if std::env::var_os(if cpp { "CXX" } else { "CC" }).is_some() {
        return Ok(None);
    }
    let exists = |name: &str| {
        (name == "cl" && cc::windows_registry::find_tool(TARGET, "cl.exe").is_some())
            || exe_on_path(name)
    };
    let (program, args): (&str, &[&str]) = match choose_windows_compiler(exists) {
        Some(WindowsCompiler::Msvc) => return Ok(None),
        Some(WindowsCompiler::Clang) => (if cpp { "clang++" } else { "clang" }, &[]),
        Some(WindowsCompiler::Gcc) => (if cpp { "g++" } else { "gcc" }, &[]),
        Some(WindowsCompiler::Zig) => ("zig", if cpp { &["c++"] } else { &["cc"] }),
        None => {
            return Err(Error::NoCompiler(
                "install one of: MSVC Build Tools, clang, gcc, or zig, and ensure it is on PATH"
                    .to_string(),
            ));
        }
    };
    Ok(Some(Compiler {
        program: program.into(),
        args: args.iter().map(Into::into).collect(),
        env: Vec::new(),
        flavor: Flavor::Gnu,
    }))
}

/// Whether `name` resolves to a runnable executable on `PATH`.
///
/// Spawns `name --version` and treats `NotFound` as absent; any other
/// outcome (success, nonzero exit, permission error) means the executable
/// exists. This respects `PATHEXT` without an extra dependency.
fn exe_on_path(name: &str) -> bool {
    match Command::new(name).arg("--version").output() {
        Ok(_) => true,
        Err(e) => e.kind() != io::ErrorKind::NotFound,
    }
}

#[cfg(test)]
mod tests;
