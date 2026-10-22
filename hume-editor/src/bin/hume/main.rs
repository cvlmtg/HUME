mod args;

use args::{Cli, Invocation, Mode, resolve};
use clap::Parser;
use std::process;

fn main() {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("hume: resolving current directory: {e}");
            process::exit(1);
        }
    };
    let Invocation { mode, config } = match resolve(Cli::parse(), &cwd) {
        Ok(inv) => inv,
        Err(msg) => {
            eprintln!("hume: {msg}");
            process::exit(1);
        }
    };
    let result = match mode {
        Mode::Headless {
            input,
            keys,
            output,
        } => hume_editor::run_keys(input, &keys, output, config, cwd),
        Mode::Normal { files } => hume_editor::run(files, config, cwd),
    };
    if let Err(e) = result {
        eprintln!("hume: {e}");
        process::exit(1);
    }
}
