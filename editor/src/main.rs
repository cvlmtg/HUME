use std::path::PathBuf;
use std::process;

fn main() {
    let file_paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();

    if let Err(e) = hume::run(file_paths) {
        eprintln!("hume: {e}");
        process::exit(1);
    }
}
