use std::path::PathBuf;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("hume {}", hume::VERSION);
        return;
    }

    let file_paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();

    if let Err(e) = hume::run(file_paths) {
        eprintln!("hume: {e}");
        process::exit(1);
    }
}
