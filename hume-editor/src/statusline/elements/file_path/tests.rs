use super::*;

// ── shorten_path_to_width_with (separator injection, cross-platform) ──────

fn windows_like_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

fn unix_like_sep(c: char) -> bool {
    c == '/'
}

#[test]
fn shorten_path_windows_seps_abbreviates_dirs() {
    // Regression for the reported bug: on Windows the path uses '\' and was
    // not being abbreviated at all, falling straight to whole-string
    // truncation ("~\foo\bar\baz\fi…"). Mirrors shorten_path_abbreviates_multiple_dirs.
    let path = r"~\foo\bar\baz.txt"; // 17 cols
    let result = shorten_path_to_width_with(path, 13, windows_like_sep);
    assert_eq!(result, r"~\f\b\baz.txt");
}

#[test]
fn shorten_path_mixed_seps_preserved() {
    // Each component keeps its own original separator character rather than
    // normalizing to '/'.
    let path = r"~\foo/bar\file.txt"; // 18 cols
    let result = shorten_path_to_width_with(path, 14, windows_like_sep);
    assert_eq!(result, r"~\f/b\file.txt");
}

#[test]
fn shorten_path_windows_seps_ellipsis_on_very_narrow() {
    // Mirrors shorten_path_ellipsis_on_very_narrow but with '\' separators —
    // the ellipsis branch must keep the '\'-prefix, not drop separators.
    let path = r"~\foo\bar\baz.txt";
    let result = shorten_path_to_width_with(path, 10, windows_like_sep);
    assert_eq!(result, r"~\f\b\baz…");
}

#[test]
fn shorten_path_unix_sep_ignores_backslash() {
    // With a Unix-style predicate, '\' is an ordinary filename character, not
    // a separator — it must never be split on, only ever truncated as part of
    // the filename content.
    let path = r"~/foo\bar.txt"; // 13 cols
    let result = shorten_path_to_width_with(path, 8, unix_like_sep);
    assert_eq!(result, r"~/foo\b…");
}
