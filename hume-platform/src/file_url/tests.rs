use super::*;

#[cfg(not(windows))]
#[test]
fn puts_the_host_in_the_authority() {
    assert_eq!(
        path_to_file_url(Path::new("/tmp/a b"), "box").as_deref(),
        Ok("file://box/tmp/a%20b")
    );
}

#[cfg(not(windows))]
#[test]
fn percent_encodes_the_host() {
    assert_eq!(
        path_to_file_url(Path::new("/tmp"), "my box").as_deref(),
        Ok("file://my%20box/tmp")
    );
}

#[test]
fn rejects_relative_path() {
    assert_eq!(
        path_to_file_url(Path::new("rel/dir"), "box"),
        Err(FileUrlError::NotAbsolute)
    );
}

#[cfg(windows)]
#[test]
fn puts_the_host_before_a_drive_letter_path() {
    assert_eq!(
        path_to_file_url(Path::new(r"D:\test"), "box").as_deref(),
        Ok("file://box/D:/test")
    );
}

#[cfg(windows)]
#[test]
fn keeps_a_unc_server_as_the_authority() {
    assert_eq!(
        path_to_file_url(Path::new(r"\\srv\share\x"), "box").as_deref(),
        Ok("file://srv/share/x")
    );
}
