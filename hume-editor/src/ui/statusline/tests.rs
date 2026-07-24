use super::elements::file_path::{display_path_string, shorten_path_to_width, shorten_path_to_width_with};
use super::*;
use ratatui::style::Style;

fn s(text: &'static str) -> (Cow<'static, str>, Style) {
    (Cow::Borrowed(text), Style::default())
}

// ── section_width ─────────────────────────────────────────────────────────

#[test]
fn section_width_empty() {
    assert_eq!(section_width(&[]), 0);
}

#[test]
fn section_width_ascii() {
    let spans = vec![s("NOR"), s(" "), s("│")];
    // "NOR"=3, " "=1, "│"=1 (U+2502 is width 1)
    assert_eq!(section_width(&spans), 5);
}

#[test]
fn section_width_cjk() {
    // CJK character is display-width 2.
    let spans = vec![s("A"), (Cow::Borrowed("中"), Style::default())];
    assert_eq!(section_width(&spans), 3);
}

// ── pad_left / pad_right ──────────────────────────────────────────────────

#[test]
fn pad_left_prepends_space() {
    let colors = crate::ui::theme::EditorColors::default();
    let spans = pad_left(vec![s("NOR")], &colors);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].0.as_ref(), " ");
    assert_eq!(spans[1].0.as_ref(), "NOR");
}

#[test]
fn pad_left_empty_is_noop() {
    let colors = crate::ui::theme::EditorColors::default();
    let spans = pad_left(vec![], &colors);
    assert!(spans.is_empty());
}

#[test]
fn pad_right_appends_space() {
    let colors = crate::ui::theme::EditorColors::default();
    let spans = pad_right(vec![s("NOR")], &colors);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].0.as_ref(), "NOR");
    assert_eq!(spans[1].0.as_ref(), " ");
}

#[test]
fn pad_right_empty_is_noop() {
    let colors = crate::ui::theme::EditorColors::default();
    let spans = pad_right(vec![], &colors);
    assert!(spans.is_empty());
}

// ── MacroRecording element ────────────────────────────────────────────────

fn test_editor() -> crate::editor::Editor {
    use crate::editor::buffer::Buffer;
    use hume_editing::{
        selection::{Selection, SelectionSet},
        text::Text,
    };
    let text = Text::from("hello\n");
    let sels = SelectionSet::single(Selection::collapsed(0));
    crate::editor::Editor::for_testing(Buffer::new(text, sels))
}

#[test]
fn macro_recording_element_idle_renders_empty() {
    // When not recording, MacroRecording should contribute an empty string.
    let ed = test_editor();
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::MacroRecording, &ed, &colors, "");
    assert!(
        text.is_empty(),
        "expected empty string when not recording, got {:?}",
        text
    );
}

#[test]
fn macro_recording_element_active_renders_label() {
    // While recording into register 'q', MacroRecording renders its label.
    let mut ed = test_editor();
    ed.state.macro_recording = Some(('q', vec![]));
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::MacroRecording, &ed, &colors, "");
    insta::assert_snapshot!(text, @"[recording @q]");
}

#[test]
fn macro_recording_element_named_register() {
    // Same but for a named register '3'.
    let mut ed = test_editor();
    ed.state.macro_recording = Some(('3', vec![]));
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::MacroRecording, &ed, &colors, "");
    insta::assert_snapshot!(text, @"[recording @3]");
}

// ── LineEnding element ────────────────────────────────────────────────────

fn test_editor_with_text(s: &str) -> crate::editor::Editor {
    use crate::editor::buffer::Buffer;
    use hume_editing::{
        selection::{Selection, SelectionSet},
        text::Text,
    };
    let text = Text::from(s);
    let sels = SelectionSet::single(Selection::collapsed(0));
    crate::editor::Editor::for_testing(Buffer::new(text, sels))
}

#[test]
fn line_ending_element_lf() {
    let ed = test_editor_with_text("hello\n");
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::LineEnding, &ed, &colors, "");
    insta::assert_snapshot!(text, @"LF");
}

#[test]
fn line_ending_element_crlf() {
    let ed = test_editor_with_text("hello\r\n");
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::LineEnding, &ed, &colors, "");
    insta::assert_snapshot!(text, @"CRLF");
}

// ── Cwd element ───────────────────────────────────────────────────────────

#[test]
fn cwd_element_renders_nonempty() {
    // Smoke test: current_dir() succeeds in a normal test run.
    let ed = test_editor();
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::Cwd, &ed, &colors, "");
    assert!(
        !text.is_empty(),
        "Cwd rendered empty; expected a path string"
    );
}

// ── Language element ──────────────────────────────────────────────────────

#[test]
fn language_element_empty_when_none() {
    let ed = test_editor();
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::Language, &ed, &colors, "");
    assert!(
        text.is_empty(),
        "expected empty string for undetected language, got {:?}",
        text
    );
}

#[test]
fn language_element_renders_bracketed() {
    let mut ed = test_editor();
    let lang = ed.state.languages.intern("rust");
    ed.doc_mut().language = Some(lang);
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::Language, &ed, &colors, "");
    insta::assert_snapshot!(text, @"[rust]");
}

// ── ReadOnly element ──────────────────────────────────────────────────────

#[test]
fn readonly_element_empty_for_normal_buffer() {
    let ed = test_editor();
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::ReadOnly, &ed, &colors, "");
    assert!(
        text.is_empty(),
        "expected empty for writable buffer, got {text:?}"
    );
}

#[test]
fn readonly_element_renders_ro_label() {
    use crate::editor::buffer::Buffer;
    let buf = Buffer::read_only_view(
        hume_editing::text::Text::from("hello\n"),
        "[test]".to_string(),
    );
    let ed = crate::editor::Editor::for_testing(buf);
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = render_element(StatusElement::ReadOnly, &ed, &colors, "");
    insta::assert_snapshot!(text, @"[RO]");
}

// ── Diagnostics element (format) ──────────────────────────────────────────
//
// `format` is deterministic (`Data` carries the spinner frame as a plain
// `usize`, no clock involved), so these exercise it directly with synthetic
// `Data` rather than through a full `Editor` fixture — see the
// `StatuslineElement` trait's doc comment. These are the appearance tests
// (exact glyphs/spacing pinned via inline snapshots); data flow driven by a
// live LSP server (the `Starting` → `Progress` → `Idle` transitions, and the
// counts themselves) is covered as data assertions in
// `editor::tests::lsp_statusline`.

use crate::editor::lsp::introspect::LspActivity;

#[test]
fn diagnostics_element_starting_shows_spinner() {
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = DiagnosticsElement::format((LspActivity::Starting, 0, 0, 0), &colors);
    insta::assert_snapshot!(text, @"⠋ lsp");
}

#[test]
fn diagnostics_element_progress_with_percentage() {
    let colors = crate::ui::theme::EditorColors::default();
    let data = (
        LspActivity::Progress {
            percentage: Some(45),
        },
        0,
        0,
        0,
    );
    let (text, _) = DiagnosticsElement::format(data, &colors);
    insta::assert_snapshot!(text, @"⠋ 45%");
}

#[test]
fn diagnostics_element_progress_with_no_percentage_shows_generic_label() {
    let colors = crate::ui::theme::EditorColors::default();
    let data = (LspActivity::Progress { percentage: None }, 0, 0, 0);
    let (text, _) = DiagnosticsElement::format(data, &colors);
    insta::assert_snapshot!(text, @"⠋ lsp");
}

#[test]
fn diagnostics_element_spinner_indexes_by_frame() {
    let colors = crate::ui::theme::EditorColors::default();
    // Frame 3 selects the 4th glyph in the SPINNER table (⠋⠙⠹⠸…).
    let (text, _) = DiagnosticsElement::format((LspActivity::Starting, 0, 0, 3), &colors);
    insta::assert_snapshot!(text, @"⠸ lsp");
}

#[test]
fn diagnostics_element_idle_shows_counts_with_no_spinner() {
    let colors = crate::ui::theme::EditorColors::default();
    let data = (LspActivity::Idle, 3, 12, 0);
    let (text, _) = DiagnosticsElement::format(data, &colors);
    insta::assert_snapshot!(text, @"✘ 3 ⚠ 12");
}

#[test]
fn diagnostics_element_omits_zero_warning_half() {
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = DiagnosticsElement::format((LspActivity::Idle, 1, 0, 0), &colors);
    insta::assert_snapshot!(text, @"✘ 1");
}

#[test]
fn diagnostics_element_omits_zero_error_half() {
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = DiagnosticsElement::format((LspActivity::Idle, 0, 1, 0), &colors);
    insta::assert_snapshot!(text, @"⚠ 1");
}

/// A `$/progress` task in flight must not hide diagnostic counts already
/// known for the buffer — the spinner and the counts render together.
#[test]
fn diagnostics_element_progress_and_counts_render_together() {
    let colors = crate::ui::theme::EditorColors::default();
    let data = (
        LspActivity::Progress {
            percentage: Some(45),
        },
        3,
        12,
        0,
    );
    let (text, _) = DiagnosticsElement::format(data, &colors);
    insta::assert_snapshot!(text, @"⠋ 45% ✘ 3 ⚠ 12");
}

#[test]
fn diagnostics_element_idle_with_no_diagnostics_is_empty() {
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = DiagnosticsElement::format((LspActivity::Idle, 0, 0, 0), &colors);
    assert!(text.is_empty(), "expected empty, got {text:?}");
}

// ── center_x arithmetic ───────────────────────────────────────────────────

#[test]
fn center_x_saturates_on_overflow() {
    // When center_w > gap, saturating_sub prevents u16 wrapping.
    // gap/2=1, center_w/2=5 → without saturating_sub this would wrap.
    let left_end: u16 = 5;
    let gap: u16 = 2;
    let center_w: u16 = 10;
    let center_x = (left_end + gap / 2).saturating_sub(center_w / 2);
    // Should not panic and should produce a value ≤ left_end (saturated to 0 at best).
    assert!(center_x <= left_end);
}

// ── shorten_path_to_width ─────────────────────────────────────────────────

#[test]
fn shorten_path_fits_unchanged() {
    // Path that already fits — returned verbatim.
    let path = "~/dev/foo.txt";
    let result = shorten_path_to_width(path, 50);
    assert_eq!(result, path);
}

#[test]
fn shorten_path_exactly_at_limit_is_unchanged() {
    let path = "~/foo/bar.txt"; // width = 13
    let result = shorten_path_to_width(path, 13);
    assert_eq!(result, path, "path at exact limit should not be shortened");
}

#[test]
fn shorten_path_abbreviates_first_dir() {
    // "~/foo/bar/baz.txt" → "~/f/bar/baz.txt" when narrowed enough.
    let path = "~/foo/bar/baz.txt";
    // Full width = 17. At 15 we expect first dir abbreviated.
    // Independent oracle: "~/f/bar/baz.txt" = 15 chars = 15 cols.
    let result = shorten_path_to_width(path, 15);
    assert_eq!(result, "~/f/bar/baz.txt");
}

#[test]
fn shorten_path_abbreviates_multiple_dirs() {
    // "~/foo/bar/baz.txt" → "~/f/b/baz.txt" when even narrower.
    let path = "~/foo/bar/baz.txt";
    // "~/f/b/baz.txt" = 13 cols.
    let result = shorten_path_to_width(path, 13);
    assert_eq!(result, "~/f/b/baz.txt");
}

#[test]
fn shorten_path_abbreviation_stops_early_when_it_fits() {
    // Should abbreviate minimally — stop as soon as it fits.
    // "~/foo/bar/baz.txt" = 17.  At budget 16: first dir abbreviated →
    // "~/f/bar/baz.txt" = 15 ≤ 16 → done (second dir NOT abbreviated).
    let path = "~/foo/bar/baz.txt";
    let result = shorten_path_to_width(path, 16);
    assert_eq!(result, "~/f/bar/baz.txt");
}

#[test]
fn shorten_path_ellipsis_on_very_narrow() {
    // When even all dirs abbreviated still too wide, truncate filename.
    // "~/foo/bar/baz.txt" fully abbreviated dirs = "~/f/b/baz.txt" = 13.
    // At budget 10: "~/f/b/" (6) + "…" (1) = 7; filename available = 3.
    // "baz" = 3, so result = "~/f/b/baz…".
    let path = "~/foo/bar/baz.txt";
    let result = shorten_path_to_width(path, 10);
    assert_eq!(result, "~/f/b/baz…");
}

#[test]
fn shorten_path_zero_budget_returns_empty() {
    // Fail oracle: if budget logic doesn't check for 0, could produce garbage.
    let result = shorten_path_to_width("~/foo/bar.txt", 0);
    assert_eq!(result, "");
}

#[test]
fn shorten_path_no_dirs_abbreviates_only_filename() {
    // Flat filename with no directory component — only ellipsis can help.
    // "readme.txt" = 10. At budget 6: need to truncate. "readm" (5) + "…" (1) = 6.
    let result = shorten_path_to_width("readme.txt", 6);
    assert_eq!(result, "readm…");
}

#[test]
fn shorten_path_tilde_alone_not_abbreviated() {
    // "~" is a 1-grapheme component and must not be shortened further.
    let path = "~/very-long-filename.txt";
    // All dirs abbreviated: "~" stays "~", filename gets truncated if needed.
    // Width of "~/very-long-filename.txt" = 24. At budget 10:
    // dirs fully abbreviated → still "~/very-long-filename.txt" (~ is the only dir)
    // Truncate filename to fit budget 10: "~/…" prefix = 3; available = 7.
    // "very-lon" = 8 > 7, "very-lo" = 7 → result = "~/very-lo…"
    let result = shorten_path_to_width(path, 10);
    assert_eq!(result, "~/very-lo…");
}

#[test]
fn shorten_path_unicode_dir_name() {
    // A CJK dir name (each char = 2 display cols). "中文" = 4 cols.
    // Path: "/中文/foo/bar.txt". Full width = 1+4+1+3+1+7 = 17.
    // At budget 13: abbreviate /中文/ to its first grapheme → "/中/foo/bar.txt"
    // = 1 + 2 + 1 + 3 + 1 + 7 = 15. Still too wide.
    // Abbreviate /foo/ → "/中/f/bar.txt" = 1+2+1+1+1+7 = 13. Fits.
    let path = "/中文/foo/bar.txt";
    let result = shorten_path_to_width(path, 13);
    assert_eq!(result, "/中/f/bar.txt");
}

#[test]
fn shorten_path_actually_abbreviates_when_too_wide() {
    // Flip-a-condition check: path would fail if we just returned input.
    let path = "~/aaaa/bbbb/cccc.txt"; // 20 cols
    let result = shorten_path_to_width(path, 15);
    assert_ne!(
        result, path,
        "path was not shortened when it should have been"
    );
    assert!(
        unicode_width::UnicodeWidthStr::width(result.as_str()) <= 15,
        "shortened path {result:?} exceeds budget of 15 cols"
    );
}

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

// ── display_path_string (Windows `\\?\` prefix stripping) ─────────────────

#[test]
fn display_path_string_none_is_empty() {
    assert_eq!(display_path_string(None), "");
}

#[test]
fn display_path_string_plain_path_matches_shorten_home() {
    // No verbatim prefix to strip: output must be exactly what shorten_home
    // alone would produce — an oracle independent of strip_unc_prefix.
    let path = std::path::Path::new("/some/absolute/path/file.rs");
    assert_eq!(
        display_path_string(Some(path)),
        hume_platform::path::shorten_home(path)
    );
}

#[cfg(windows)]
#[test]
fn display_path_string_strips_windows_verbatim_prefix() {
    // Regression: statusline must never show the raw `\\?\` extended-length
    // prefix that `canonicalize` attaches on Windows.
    let path = std::path::Path::new(r"\\?\C:\Users\x\file.rs");
    let result = display_path_string(Some(path));
    assert!(
        !result.contains(r"\\?\"),
        "statusline path {result:?} still carries the verbatim prefix"
    );
    assert!(result.contains("file.rs"));
}
