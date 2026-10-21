use super::*;

#[test]
fn choose_windows_compiler_prefers_msvc_when_present() {
    let choice = choose_windows_compiler(|name| matches!(name, "cl" | "clang" | "gcc" | "zig"));
    assert_eq!(choice, Some(WindowsCompiler::Msvc));
}

#[test]
fn choose_windows_compiler_falls_back_to_clang() {
    let choice = choose_windows_compiler(|name| matches!(name, "clang" | "gcc" | "zig"));
    assert_eq!(choice, Some(WindowsCompiler::Clang));
}

#[test]
fn choose_windows_compiler_falls_back_to_gcc() {
    let choice = choose_windows_compiler(|name| matches!(name, "gcc" | "zig"));
    assert_eq!(choice, Some(WindowsCompiler::Gcc));
}

#[test]
fn choose_windows_compiler_falls_back_to_zig() {
    let choice = choose_windows_compiler(|name| name == "zig");
    assert_eq!(choice, Some(WindowsCompiler::Zig));
}

#[test]
fn choose_windows_compiler_none_when_nothing_present() {
    let choice = choose_windows_compiler(|_| false);
    assert_eq!(choice, None);
}
