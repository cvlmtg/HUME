//! Compiles a tree-sitter grammar into the shared library HUME loads: the
//! generated `parser.c`, plus an external scanner written in C or C++.

#![deny(rustdoc::broken_intra_doc_links)]

mod invocation;
mod toolchain;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use toolchain::Compiler;

/// A grammar's sources, as `tree-sitter generate` lays them out in `src/`.
#[derive(Debug)]
struct Sources {
    /// The include root for `tree_sitter/parser.h`.
    dir: PathBuf,
    parser: PathBuf,
    scanner: Option<Scanner>,
}

/// An external scanner, by the language it is written in.
#[derive(Debug)]
enum Scanner {
    C(PathBuf),
    Cpp(PathBuf),
}

impl Sources {
    /// `scanner.c` is used when a grammar ships both scanners, as the
    /// tree-sitter CLI does.
    fn find(grammar_dir: &Path) -> Result<Self, Error> {
        let dir = grammar_dir.join("src");
        let parser = dir.join("parser.c");
        if !parser.is_file() {
            return Err(Error::NoParser(parser));
        }
        let c = dir.join("scanner.c");
        let cpp = dir.join("scanner.cc");
        let scanner = if c.is_file() {
            Some(Scanner::C(c))
        } else if cpp.is_file() {
            Some(Scanner::Cpp(cpp))
        } else {
            None
        };
        Ok(Self {
            dir,
            parser,
            scanner,
        })
    }
}

/// Why a grammar did not build.
#[derive(Debug)]
pub enum Error {
    NoParser(PathBuf),
    NoFileName(PathBuf),
    NoCompiler(String),
    Spawn {
        program: PathBuf,
        source: io::Error,
    },
    Failed {
        program: PathBuf,
        status: ExitStatus,
    },
    Io {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoParser(path) => write!(f, "no grammar parser at {}", path.display()),
            Self::NoFileName(path) => write!(f, "output path {} names no file", path.display()),
            Self::NoCompiler(reason) => write!(f, "no usable compiler: {reason}"),
            Self::Spawn { program, source } => {
                write!(f, "cannot run {}: {source}", program.display())
            }
            Self::Failed { program, status } => {
                write!(f, "{} failed ({status})", program.display())
            }
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

/// Build the grammar rooted at `grammar_dir` into the shared library `out`.
///
/// The library is linked in a scratch directory beside `out` and renamed
/// over it, so `out` is never a partly written file: a failed rebuild leaves
/// the previous library loadable.
pub fn build(grammar_dir: &Path, out: &Path) -> Result<(), Error> {
    let absolute = |path: &Path| {
        std::path::absolute(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
    };
    let grammar_dir = absolute(grammar_dir)?;
    let out = absolute(out)?;
    let file_name = out
        .file_name()
        .ok_or_else(|| Error::NoFileName(out.clone()))?;
    let out_dir = out.parent().ok_or_else(|| Error::NoFileName(out.clone()))?;

    let sources = Sources::find(&grammar_dir)?;
    let compiler = Compiler::find(matches!(sources.scanner, Some(Scanner::Cpp(_))))?;

    let io_error = |source| Error::Io {
        path: out_dir.to_path_buf(),
        source,
    };
    std::fs::create_dir_all(out_dir).map_err(io_error)?;
    let work = tempfile::Builder::new()
        .prefix(".hume-grammar-build")
        .tempdir_in(out_dir)
        .map_err(io_error)?;
    let library = work.path().join(file_name);
    for args in invocation::steps(compiler.flavor(), &sources, work.path(), &library) {
        compiler.run(&args, work.path())?;
    }
    std::fs::rename(&library, &out).map_err(|source| Error::Io { path: out, source })
}

#[cfg(test)]
mod tests;
