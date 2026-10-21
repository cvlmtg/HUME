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

#[test]
fn fallback_driver_leaves_msvc_to_cc() {
    assert_eq!(fallback_driver(WindowsCompiler::Msvc, false), None);
    assert_eq!(fallback_driver(WindowsCompiler::Msvc, true), None);
}

#[test]
fn fallback_drivers_name_the_language_driver_and_optimize() {
    let cases = [
        (WindowsCompiler::Clang, false, "clang", vec!["-O2"]),
        (WindowsCompiler::Clang, true, "clang++", vec!["-O2"]),
        (WindowsCompiler::Gcc, false, "gcc", vec!["-O2"]),
        (
            WindowsCompiler::Gcc,
            true,
            "g++",
            vec!["-O2", "-static-libgcc", "-static-libstdc++"],
        ),
        (WindowsCompiler::Zig, false, "zig", vec!["cc", "-O2"]),
        (WindowsCompiler::Zig, true, "zig", vec!["c++", "-O2"]),
    ];
    for (compiler, cpp, program, args) in cases {
        assert_eq!(
            fallback_driver(compiler, cpp),
            Some((program, args)),
            "{compiler:?} cpp={cpp}"
        );
    }
}
