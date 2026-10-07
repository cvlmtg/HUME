use super::*;

#[test]
fn runtime_dir_respects_hume_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    let rt = tmp.path().to_string_lossy().into_owned();
    let result = runtime_dir_with(
        |k| match k {
            "HUME_RUNTIME" => Some(rt.clone()),
            _ => None,
        },
        None,
        None,
    );
    assert_eq!(result, Some(tmp.path().to_path_buf()));
}

#[test]
fn runtime_dir_hume_runtime_wins_over_exe_and_cwd() {
    let exe_tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(exe_tmp.path().join("runtime")).unwrap();
    let cwd_tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(cwd_tmp.path().join("runtime")).unwrap();
    let override_tmp = tempfile::tempdir().unwrap();
    let override_str = override_tmp.path().to_string_lossy().into_owned();

    let result = runtime_dir_with(
        |k| match k {
            "HUME_RUNTIME" => Some(override_str.clone()),
            _ => None,
        },
        Some(exe_tmp.path().join("hume")),
        Some(cwd_tmp.path().to_path_buf()),
    );
    assert_eq!(result, Some(override_tmp.path().to_path_buf()));
}

#[test]
fn runtime_dir_falls_back_to_exe_relative_runtime() {
    // Portable/archive layout: hume(.exe) sits next to runtime/ (nightly zips).
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("runtime")).unwrap();

    let result = runtime_dir_with(|_| None, Some(tmp.path().join("hume")), None);
    assert_eq!(result, Some(tmp.path().join("runtime")));
}

#[test]
fn runtime_dir_falls_back_to_cwd_relative_runtime() {
    // Dev layout: cargo run produces target/…/hume; runtime/ sits at the cwd
    // (workspace root), unrelated to the exe path.
    let exe_tmp = tempfile::tempdir().unwrap(); // no runtime/ next to the exe
    let cwd_tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(cwd_tmp.path().join("runtime")).unwrap();

    let result = runtime_dir_with(
        |_| None,
        Some(exe_tmp.path().join("hume")),
        Some(cwd_tmp.path().to_path_buf()),
    );
    assert_eq!(result, Some(cwd_tmp.path().join("runtime")));
}

#[test]
fn runtime_dir_none_when_no_candidate_exists() {
    let exe_tmp = tempfile::tempdir().unwrap();
    let cwd_tmp = tempfile::tempdir().unwrap();

    let result = runtime_dir_with(
        |_| None,
        Some(exe_tmp.path().join("hume")),
        Some(cwd_tmp.path().to_path_buf()),
    );
    assert_eq!(result, None);
}

#[test]
fn resolve_joins_a_relative_runtime_and_tmp_onto_the_startup_cwd() {
    let cwd = tempfile::tempdir().unwrap();
    let dirs = Dirs::resolve(
        |k| match k {
            "HUME_RUNTIME" => Some("rt".to_string()),
            _ => None,
        },
        None,
        Some(cwd.path().to_path_buf()),
        PathBuf::from("scratch"),
    );
    assert_eq!(dirs.runtime, Some(cwd.path().join("rt")));
    assert_eq!(dirs.tmp, Some(cwd.path().join("scratch")));
}

#[test]
fn resolve_keeps_absolute_runtime_and_tmp_as_they_are() {
    let cwd = tempfile::tempdir().unwrap();
    let rt = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rt_str = rt.path().to_string_lossy().into_owned();
    let dirs = Dirs::resolve(
        |k| match k {
            "HUME_RUNTIME" => Some(rt_str.clone()),
            _ => None,
        },
        None,
        Some(cwd.path().to_path_buf()),
        tmp.path().to_path_buf(),
    );
    assert_eq!(dirs.runtime, Some(rt.path().to_path_buf()));
    assert_eq!(dirs.tmp, Some(tmp.path().to_path_buf()));
}

#[test]
fn resolve_without_a_cwd_leaves_relative_values_as_given() {
    let dirs = Dirs::resolve(
        |k| match k {
            "HUME_RUNTIME" => Some("rt".to_string()),
            _ => None,
        },
        None,
        None,
        PathBuf::from("scratch"),
    );
    assert_eq!(dirs.runtime, Some(PathBuf::from("rt")));
    assert_eq!(dirs.tmp, Some(PathBuf::from("scratch")));
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
