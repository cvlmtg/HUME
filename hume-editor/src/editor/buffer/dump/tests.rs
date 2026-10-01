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
