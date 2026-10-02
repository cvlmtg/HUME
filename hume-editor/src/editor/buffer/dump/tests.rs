use super::*;
use std::path::Path;

#[test]
fn dump_path_appends_the_suffix_to_the_full_file_name() {
    assert_eq!(
        dump_path_for(Path::new("/work/foo.txt")),
        Path::new("/work/foo.txt.dump")
    );
}

#[test]
fn dump_path_keeps_an_extensionless_name_and_its_directory() {
    assert_eq!(
        dump_path_for(Path::new("rel/dir/Makefile")),
        Path::new("rel/dir/Makefile.dump")
    );
}

#[test]
fn dump_path_of_a_dump_stacks_the_suffix() {
    assert_eq!(
        dump_path_for(Path::new("a.txt.dump")),
        Path::new("a.txt.dump.dump")
    );
}

#[test]
fn a_panicking_write_becomes_an_error_carrying_the_panic_message() {
    let outcome: io::Result<()> = panic_to_error(|| panic!("rope invariant broken"));

    let err = outcome.unwrap_err();
    assert!(
        err.to_string().contains("rope invariant broken"),
        "got: {err}"
    );
}

#[test]
fn a_panicking_write_with_a_formatted_message_keeps_the_message() {
    let outcome: io::Result<()> = panic_to_error(|| panic!("bad offset {}", 42));

    assert!(outcome.unwrap_err().to_string().contains("bad offset 42"));
}

#[test]
fn a_write_that_does_not_panic_passes_its_result_through() {
    assert_eq!(panic_to_error(|| Ok(5)).unwrap(), 5);
    let failed: io::Result<()> = panic_to_error(|| Err(io::Error::other("disk full")));
    assert_eq!(failed.unwrap_err().to_string(), "disk full");
}

fn one_saved_one_failed() -> Vec<(String, io::Result<PathBuf>)> {
    vec![
        ("foo.txt".to_string(), Ok(PathBuf::from("/w/foo.txt.dump"))),
        (
            "*scratch*".to_string(),
            Err(io::Error::other("no data directory")),
        ),
    ]
}

#[test]
fn the_report_names_each_buffer_and_where_it_went() {
    let mut out = Vec::new();

    report_dumps(&mut out, &one_saved_one_failed());

    insta::assert_snapshot!(String::from_utf8(out).unwrap(), @r"
    hume: unsaved foo.txt saved to /w/foo.txt.dump
    hume: could not save unsaved *scratch*: no data directory
    ");
}

#[test]
fn the_report_keeps_going_when_the_writer_fails() {
    struct Broken(usize);
    impl io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.0 += 1;
            Err(io::Error::other("stderr is gone"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut out = Broken(0);
    let outcomes = one_saved_one_failed();

    report_dumps(&mut out, &outcomes);

    assert_eq!(out.0, outcomes.len(), "each line is attempted once");
}
