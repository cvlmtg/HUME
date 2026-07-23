//! Windows-only tests, gated once at the `mod windows;`
//! declaration in the parent.

use super::*;

#[test]
fn home_dir_uses_userprofile() {
    let result = home_dir_with(|k| match k {
        "USERPROFILE" => Some(r"C:\Users\Alice".to_owned()),
        _ => None,
    });
    assert_eq!(result, Some(PathBuf::from(r"C:\Users\Alice")));
}

#[test]
fn home_dir_falls_back_to_homedrive_homepath() {
    let result = home_dir_with(|k| match k {
        "HOMEDRIVE" => Some("C:".to_owned()),
        "HOMEPATH" => Some(r"\Users\Alice".to_owned()),
        _ => None,
    });
    assert_eq!(result, Some(PathBuf::from(r"C:\Users\Alice")));
}

#[test]
fn home_dir_none_when_all_unset() {
    let result = home_dir_with(|_| None);
    assert_eq!(result, None);
}
