use std::path::Path;

use crate::editor::cwd_report::working_directory_url;

#[cfg(not(windows))]
#[test]
fn url_carries_the_host_and_percent_encodes_the_path() {
    assert_eq!(
        working_directory_url("box", Path::new("/tmp/a b")).as_deref(),
        Some("file://box/tmp/a%20b")
    );
}

#[cfg(windows)]
#[test]
fn url_puts_the_host_before_a_drive_letter_path() {
    assert_eq!(
        working_directory_url("box", Path::new(r"D:\test")).as_deref(),
        Some("file://box/D:/test")
    );
}

#[cfg(windows)]
#[test]
fn url_keeps_a_unc_server_as_the_authority() {
    assert_eq!(
        working_directory_url("box", Path::new(r"\\srv\share\x")).as_deref(),
        Some("file://srv/share/x")
    );
}

#[test]
fn relative_path_has_no_url() {
    assert_eq!(working_directory_url("box", Path::new("rel/dir")), None);
}
