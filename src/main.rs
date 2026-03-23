use std::path::PathBuf;
use std::process;

fn main() {
    // Optional first argument is the file to open.
    let file_path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);

    if let Err(e) = hume::run(file_path) {
        eprintln!("hume: {e}");
        process::exit(1);
    }
}
