//! Windows-only tests, gated once at the `mod windows;`
//! declaration in the parent.

use super::*;

/// `cl` on PATH means MSVC is usable — no override, regardless of what
/// else is installed.
#[test]
fn choose_windows_compiler_prefers_msvc_when_present() {
    let choice = choose_windows_compiler(|name| matches!(name, "cl" | "clang" | "gcc" | "zig"));
    assert_eq!(choice, None, "cl present should mean no CC/CXX override");
}

#[test]
fn choose_windows_compiler_falls_back_to_clang() {
    let choice = choose_windows_compiler(|name| matches!(name, "clang" | "gcc" | "zig"));
    assert_eq!(
        choice,
        Some(WindowsCompiler::Clang),
        "clang should win over gcc/zig when cl is absent"
    );
}

#[test]
fn choose_windows_compiler_falls_back_to_gcc() {
    let choice = choose_windows_compiler(|name| matches!(name, "gcc" | "zig"));
    assert_eq!(
        choice,
        Some(WindowsCompiler::Gcc),
        "gcc should win over zig when cl/clang are absent"
    );
}

#[test]
fn choose_windows_compiler_falls_back_to_zig() {
    let choice = choose_windows_compiler(|name| name == "zig");
    assert_eq!(
        choice,
        Some(WindowsCompiler::Zig),
        "zig should be used when no other compiler is present"
    );
}

#[test]
fn choose_windows_compiler_none_when_nothing_present() {
    let choice = choose_windows_compiler(|_| false);
    assert_eq!(choice, None, "no compiler on PATH should mean no override");
}

/// Pin down the wrapper's exact contents: the `--target` strip (needed by
/// both `clang` and `zig cc`, see `target_stripping_wrapper_script`) and,
/// for `zig`, the whitespace-splitting issue in `cc`'s handling of
/// `CC="zig cc"` (see `compiler_env_vars`).
#[test]
fn target_stripping_wrapper_script_forwards_args_to_invocation() {
    let expected = "@echo off\r\n\
         setlocal enabledelayedexpansion\r\n\
         set \"ARGS=\"\r\n\
         :loop\r\n\
         if \"%~1\"==\"\" goto run\r\n\
         set \"TOK=%~1\"\r\n\
         if /i \"!TOK:~0,9!\"==\"--target=\" (shift & goto loop)\r\n\
         if /i \"!TOK!\"==\"--target\" (shift & shift & goto loop)\r\n\
         if /i \"!TOK!\"==\"-target\" (shift & shift & goto loop)\r\n\
         set \"ARGS=!ARGS! %1\"\r\n\
         shift\r\n\
         goto loop\r\n\
         :run\r\n\
         zig cc !ARGS!\r\n";
    assert_eq!(target_stripping_wrapper_script("zig cc"), expected);
    assert_eq!(
        target_stripping_wrapper_script("zig c++"),
        expected.replace("zig cc !ARGS!", "zig c++ !ARGS!")
    );
    assert_eq!(
        target_stripping_wrapper_script("clang"),
        expected.replace("zig cc !ARGS!", "clang !ARGS!")
    );
    assert_eq!(
        target_stripping_wrapper_script("clang++"),
        expected.replace("zig cc !ARGS!", "clang++ !ARGS!")
    );
}

