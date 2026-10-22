//! `hume-grammar-build -o <library> <grammar-dir>`: HUME runs this to
//! compile a grammar it installs. It builds the generated `parser.c`, plus
//! an external scanner written in C or C++, into the shared library HUME
//! loads.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

mod invocation;
mod toolchain;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};

use toolchain::Compiler;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [flag, out, grammar_dir] = args.as_slice() else {
        return usage();
    };
    if flag != "-o" {
        return usage();
    }
    match build(Path::new(grammar_dir), Path::new(out)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hume-grammar-build: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: hume-grammar-build -o <library> <grammar-dir>");
    ExitCode::from(2)
}

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
    /// The scanner is the first of `scanner.c`, `scanner.cc` and
    /// `scanner.cpp` that the grammar ships.
    fn find(grammar_dir: &Path) -> Result<Self, Error> {
        let dir = grammar_dir.join("src");
        let parser = dir.join("parser.c");
        if !parser.is_file() {
            return Err(Error::NoParser(parser));
        }
        let scanner = [
            ("scanner.c", Scanner::C as fn(PathBuf) -> Scanner),
            ("scanner.cc", Scanner::Cpp),
            ("scanner.cpp", Scanner::Cpp),
        ]
        .into_iter()
        .map(|(name, scanner)| (dir.join(name), scanner))
        .find(|(path, _)| path.is_file())
        .map(|(path, scanner)| scanner(path));
        Ok(Self {
            dir,
            parser,
            scanner,
        })
    }
}

/// Why a grammar did not build.
#[derive(Debug)]
enum Error {
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

/// An [`Error::Io`] on `path`, for `map_err`.
fn io_error(path: &Path) -> impl FnOnce(io::Error) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Io { path, source }
}

/// Build the grammar rooted at `grammar_dir` into the shared library `out`.
///
/// The library is linked in a scratch directory beside `out` and renamed
/// over it, so `out` is never a partly written file: a failed rebuild leaves
/// the previous library loadable.
fn build(grammar_dir: &Path, out: &Path) -> Result<(), Error> {
    let grammar_dir = std::path::absolute(grammar_dir).map_err(io_error(grammar_dir))?;
    let out = std::path::absolute(out).map_err(io_error(out))?;
    let file_name = out
        .file_name()
        .ok_or_else(|| Error::NoFileName(out.clone()))?;
    let out_dir = out.parent().ok_or_else(|| Error::NoFileName(out.clone()))?;

    let sources = Sources::find(&grammar_dir)?;
    let compiler = Compiler::find(matches!(sources.scanner, Some(Scanner::Cpp(_))))?;

    std::fs::create_dir_all(out_dir).map_err(io_error(out_dir))?;
    let work = tempfile::Builder::new()
        .prefix(".hume-grammar-build")
        .tempdir_in(out_dir)
        .map_err(io_error(out_dir))?;
    let library = work.path().join(file_name);
    for args in invocation::steps(compiler.flavor(), &sources, work.path(), &library) {
        compiler.run(&args, work.path())?;
    }
    std::fs::rename(&library, &out).map_err(io_error(&out))
}

#[cfg(test)]
mod tests;
