use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Error;

/// The triple this binary was built for, which is also the one it builds
/// grammars for: they are loaded into a HUME running on the same machine.
const TARGET: &str = env!("HUME_GRAMMAR_BUILD_TARGET");

/// How a compiler driver spells its flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flavor {
    Gnu,
    Msvc,
}

/// A C or C++ compiler driver, with the arguments and environment it runs with.
pub(crate) struct Compiler {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    flavor: Flavor,
}

impl Compiler {
    /// The C++ driver when `cpp`, else the C driver. `CC`/`CXX` override the
    /// platform default, as they do for `cc`'s build scripts.
    pub(crate) fn find(cpp: bool) -> Result<Self, Error> {
        #[cfg(windows)]
        if let Some(compiler) = windows::without_msvc(cpp)? {
            return Ok(compiler);
        }
        let tool = cc::Build::new()
            .cpp(cpp)
            .cargo_metadata(false)
            .cargo_warnings(false)
            .debug(false)
            .opt_level(2)
            .warnings(false)
            .host(TARGET)
            .target(TARGET)
            .try_get_compiler()
            .map_err(|e| Error::NoCompiler(e.to_string()))?;
        Ok(Self {
            program: tool.path().to_path_buf(),
            args: tool.args().to_vec(),
            env: tool.env().to_vec(),
            flavor: if tool.is_like_msvc() {
                Flavor::Msvc
            } else {
                Flavor::Gnu
            },
        })
    }

    pub(crate) fn flavor(&self) -> Flavor {
        self.flavor
    }

    /// Run the driver with `args` after its own, in `cwd`, inheriting stdio.
    pub(crate) fn run(&self, args: &[OsString], cwd: &Path) -> Result<(), Error> {
        let status = Command::new(&self.program)
            .args(&self.args)
            .args(args)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .current_dir(cwd)
            .status()
            .map_err(|source| Error::Spawn {
                program: self.program.clone(),
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Failed {
                program: self.program.clone(),
                status,
            })
        }
    }
}

#[cfg(windows)]
mod windows;
