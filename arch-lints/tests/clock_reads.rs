//! # Clock reads go through the editor's clock
//!
//! A test can hold time still or move it forward only if the code under
//! test reads the time from something the test can set. The editor reads it
//! from its `Clock` (`hume-editor/src/editor/clock.rs`), and a library crate
//! takes the current time as a `now` parameter from its caller.
//!
//! `no_direct_clock_reads` scans the workspace's non-test source for
//! `Instant::now()`, `SystemTime::now()` and `.elapsed()`, which reads the
//! system clock too. Exempt by path, since their clock reads time real
//! waits:
//!
//! - `hume-editor/src/editor/clock.rs`: the clock itself.
//! - `hume-platform/src`: waits on OS processes and the terminal.
//! - `hume-scripting/src/watchdog.rs`: the budget thread's deadline.
//! - `test-fixtures/src`: measures how long real work takes.
//!
//! **Opt-out**: `// real-clock: <reason>`, for one read that times a real
//! wait on a process or thread.

use arch_lints::{scan_lines, workspace_root, workspace_source_paths};

const OPT_OUT_MARKER: &str = "// real-clock:";
const READS: &[&str] = &["Instant::now()", "SystemTime::now()", ".elapsed()"];

fn clock_reads(line: &str) -> usize {
    READS.iter().map(|read| line.matches(read).count()).sum()
}

#[test]
fn clock_reads_counts_every_read() {
    assert_eq!(clock_reads("let a = Instant::now();"), 1);
    assert_eq!(
        clock_reads("f(SystemTime::now(), std::time::Instant::now())"),
        2
    );
    assert_eq!(clock_reads("while started.elapsed() < grace {"), 1);
    assert_eq!(clock_reads("let now = clock.now();"), 0);
    assert_eq!(clock_reads("fn next_wake(&self, now: Instant)"), 0);
}

/// A direct clock read outside the exempt paths is reported with its file
/// and line.
#[test]
fn no_direct_clock_reads() {
    let workspace_root = workspace_root();
    let exempt = [
        workspace_root.join("hume-editor/src/editor/clock.rs"),
        workspace_root.join("hume-platform/src"),
        workspace_root.join("hume-scripting/src/watchdog.rs"),
        workspace_root.join("test-fixtures/src"),
    ];
    let paths: Vec<_> = workspace_source_paths(&workspace_root, &[])
        .into_iter()
        .filter(|p| !exempt.iter().any(|e| p.starts_with(e)))
        .collect();
    assert!(!paths.is_empty(), "the scan found no source files");

    for path in &paths {
        let src = std::fs::read_to_string(path).expect("readable source");
        for (index, line) in src.lines().enumerate() {
            if let Some((_, reason)) = line.split_once(OPT_OUT_MARKER) {
                assert!(
                    !reason.trim().is_empty(),
                    "{}:{}: an opt-out names the real wait it times",
                    path.display(),
                    index + 1
                );
            }
        }
    }

    let violations = scan_lines(&paths, &workspace_root, OPT_OUT_MARKER, clock_reads);

    assert!(
        violations.is_empty(),
        "\nA direct clock read: a test cannot hold or move this time.\n\
         In the editor, read `state.clock`; in a library crate, take `now` \
         from the caller.\n\
         Opt out with `// real-clock: <reason>` only for a read that times a \
         real wait on a process or thread.\n\
         Violations:\n{}\n",
        violations
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
}
