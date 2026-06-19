use std::path::PathBuf;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("hume {}", hume_editor::VERSION);
        return;
    }

    // Headless key-runner: `hume --keys <STREAM> --output <OUT> <INPUT>`
    //
    // Opens INPUT, feeds STREAM through the editor's normal dispatch path,
    // writes the final buffer to OUT. No terminal is initialised. Used by the
    // golf harness (`tools/golf/golf.sh`) to score editing challenges.
    if let Some(keys_pos) = args.iter().position(|a| a == "--keys") {
        let keys = args.get(keys_pos + 1).unwrap_or_else(|| {
            eprintln!("hume: --keys requires a value");
            process::exit(1);
        });
        let output = args
            .iter()
            .position(|a| a == "--output")
            .and_then(|p| args.get(p + 1))
            .unwrap_or_else(|| {
                eprintln!("hume: --keys requires --output <path>");
                process::exit(1);
            });
        // Collect non-flag args as the input path (exactly one required).
        let inputs: Vec<&str> = args
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                !a.starts_with("--")
                    && args.get(i.wrapping_sub(1)).map(|s| s.as_str()) != Some("--keys")
                    && args.get(i.wrapping_sub(1)).map(|s| s.as_str()) != Some("--output")
            })
            .map(|(_, a)| a.as_str())
            .collect();
        if inputs.len() != 1 {
            eprintln!("hume: --keys mode requires exactly one input file");
            process::exit(1);
        }
        if let Err(e) =
            hume_editor::run_keys(PathBuf::from(inputs[0]), keys, PathBuf::from(output))
        {
            eprintln!("hume: {e}");
            process::exit(1);
        }
        return;
    }

    let file_paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();

    if let Err(e) = hume_editor::run(file_paths) {
        eprintln!("hume: {e}");
        process::exit(1);
    }
}
