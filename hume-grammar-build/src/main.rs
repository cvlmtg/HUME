//! `hume-grammar-build -o <library> <grammar-dir>`: HUME runs this to
//! compile a grammar it installs.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [flag, out, grammar_dir] = args.as_slice() else {
        return usage();
    };
    if flag != "-o" {
        return usage();
    }
    match hume_grammar_build::build(Path::new(grammar_dir), Path::new(out)) {
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
