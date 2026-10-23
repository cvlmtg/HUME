use std::path::Path;
use std::time::Duration;

use super::*;
use crate::wake::counting_wake;

#[cfg(unix)]
use unix::write_probe;
#[cfg(windows)]
use windows::write_probe;

/// The programs in `dirs`, scanned in order.
fn programs_in(dirs: &[&Path], prefix: &str, extensions: &[&str]) -> Vec<String> {
    let mut names = ProgramNames::default();
    for dir in dirs {
        names.scan_dir(dir, prefix, extensions);
    }
    names.sorted().to_vec()
}

/// `dirs` joined into one `PATH` value.
fn search_path(dirs: &[&Path]) -> std::ffi::OsString {
    std::env::join_paths(dirs).unwrap()
}

/// Reads `scan` until it ends, failing the test after a second without a
/// wake.
fn scan_to_end(
    mut scan: Snapshots<Arc<[String]>>,
    woken: &std::sync::mpsc::Receiver<()>,
) -> Vec<String> {
    loop {
        woken
            .recv_timeout(Duration::from_secs(1))
            .expect("the scan wakes");
        if let Some(progress) = scan.try_take()
            && progress.done
        {
            return progress.value.to_vec();
        }
    }
}

#[test]
fn a_missing_directory_and_a_directory_entry_list_nothing() {
    let bin = tempfile::tempdir().unwrap();
    std::fs::create_dir(bin.path().join("probe.exe")).unwrap();
    let missing = bin.path().join("missing");
    assert_eq!(
        programs_in(&[&missing, bin.path()], "", &[".exe"]),
        Vec::<String>::new()
    );
}

#[test]
fn scan_dir_reports_whether_it_added_a_program() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    write_probe(first.path(), "probe");
    write_probe(second.path(), "probe");
    write_probe(second.path(), "other");
    let mut names = ProgramNames::default();
    assert!(names.scan_dir(first.path(), "", &[".exe"]));
    assert!(!names.scan_dir(second.path(), "p", &[".exe"]));
    assert!(names.scan_dir(second.path(), "", &[".exe"]));
    assert_eq!(names.sorted().to_vec(), ["other", "probe"]);
}

#[test]
fn a_program_scan_ends_with_every_directorys_programs_sorted() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    write_probe(first.path(), "probe-zeta");
    write_probe(second.path(), "probe-alpha");
    write_probe(second.path(), "other");
    let (wake, woken) = counting_wake();
    let scan = spawn_program_scan(
        Some(search_path(&[first.path(), second.path()])),
        "probe".to_owned(),
        vec![".exe".to_owned()],
        wake,
    )
    .expect("spawn");
    assert_eq!(scan_to_end(scan, &woken), ["probe-alpha", "probe-zeta"]);
}

#[test]
fn a_program_scan_without_a_search_path_ends_empty() {
    let (wake, woken) = counting_wake();
    let scan = spawn_program_scan(None, String::new(), Vec::new(), wake).expect("spawn");
    assert_eq!(scan_to_end(scan, &woken), Vec::<String>::new());
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
