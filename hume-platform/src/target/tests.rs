use super::hume_target;

#[test]
fn returns_one_of_the_known_targets_or_none() {
    match hume_target() {
        None => {}
        Some(t) => assert!(
            matches!(
                t,
                "darwin-arm64" | "darwin-x64" | "linux-x64" | "windows-x64"
            ),
            "unexpected hume-target value: {t}"
        ),
    }
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn macos_aarch64_is_darwin_arm64() {
    assert_eq!(hume_target(), Some("darwin-arm64"));
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn macos_x86_64_is_darwin_x64() {
    assert_eq!(hume_target(), Some("darwin-x64"));
}

#[test]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linux_x86_64_is_linux_x64() {
    assert_eq!(hume_target(), Some("linux-x64"));
}

#[test]
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn windows_x86_64_is_windows_x64() {
    assert_eq!(hume_target(), Some("windows-x64"));
}
