//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;

#[test]
fn config_dir_respects_xdg_config_home() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg = tmp.path().to_string_lossy().into_owned();
    let result = config_dir_with(|k| match k {
        "XDG_CONFIG_HOME" => Some(xdg.clone()),
        _ => None,
    });
    assert_eq!(result, Some(tmp.path().join("hume")));
}

#[test]
fn data_dir_respects_xdg_data_home() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg = tmp.path().to_string_lossy().into_owned();
    let result = data_dir_with(|k| match k {
        "XDG_DATA_HOME" => Some(xdg.clone()),
        _ => None,
    });
    assert_eq!(result, Some(tmp.path().join("hume")));
}

#[test]
fn runtime_dir_prefers_installed_share_layout_over_exe_relative_runtime() {
    // Installed FHS layout: <prefix>/bin/hume -> <prefix>/share/hume, even when an
    // exe-relative runtime/ also happens to exist (share/ wins).
    let prefix = tempfile::tempdir().unwrap();
    let bin_dir = prefix.path().join("bin");
    std::fs::create_dir(&bin_dir).unwrap();
    std::fs::create_dir(bin_dir.join("runtime")).unwrap();
    let share_dir = prefix.path().join("share").join("hume");
    std::fs::create_dir_all(&share_dir).unwrap();

    let result = runtime_dir_with(|_| None, Some(bin_dir.join("hume")), None);
    assert_eq!(result, Some(share_dir));
}

#[test]
fn home_dir_uses_home_env() {
    let result = home_dir_with(|k| match k {
        "HOME" => Some("/home/alice".to_owned()),
        _ => None,
    });
    assert_eq!(result, Some(PathBuf::from("/home/alice")));
}

#[test]
fn home_dir_none_when_home_unset() {
    let result = home_dir_with(|_| None);
    assert_eq!(result, None);
}

