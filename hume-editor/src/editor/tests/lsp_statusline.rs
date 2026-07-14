// Statusline diagnostics element: `StatusElement::Diagnostics`
// reads the diagnostics store directly (never through Steel) and renders
// `"✘ E ⚠ W"`, omitting either half when its count is zero and collapsing
// to empty when both counts are zero.

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use crate::ui::statusline::{DIAGNOSTICS_ERROR_GLYPH, DIAGNOSTICS_WARNING_GLYPH, StatusElement};
use hume_lsp::backend::LspBackend;
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;

/// `((start_line, start_char), (end_line, end_char), severity)`.
type DiagFixture = ((u32, u32), (u32, u32), i64);

fn publish_diagnostics_notification(
    uri: &str,
    ranges_and_severity: &[DiagFixture],
) -> hume_lsp::codec::Message {
    let diagnostics: Vec<serde_json::Value> = ranges_and_severity
        .iter()
        .map(|&((sl, sc), (el, ec), severity)| {
            serde_json::json!({
                "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
                "severity": severity,
                "message": "boom",
            })
        })
        .collect();
    hume_lsp::codec::Message::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::json!({"uri": uri, "diagnostics": diagnostics}),
    }
}

struct DiagCtx {
    _file_dir: tempfile::TempDir,
    ed: Editor,
}

/// `publishes`: one or more diagnostic batches for the same file, pushed to
/// the scripted backend in order *before* the single `drain_lsp()` call —
/// multiple publishes in one drain batch coalesce to the last one (same
/// semantics `lsp_diagnostics.rs`'s
/// `two_publishes_in_one_drain_batch_coalesce_to_the_last` locks in), which
/// is exactly the "server republishes with the error fixed" scenario.
fn setup(content: &str, publishes: &[&[DiagFixture]]) -> DiagCtx {
    let file_dir = tempfile::tempdir().unwrap();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, content).unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    for diags in publishes {
        backend.push_from_server(sid, publish_diagnostics_notification(uri.as_str(), diags));
    }

    let mut ed = Editor::open(None).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, std::path::PathBuf::from(".")));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    ed.drain_lsp();

    DiagCtx {
        _file_dir: file_dir,
        ed,
    }
}

#[test]
fn diagnostics_element_empty_with_no_diagnostics() {
    let c = setup("abcdefgh\n", &[]);
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) =
        crate::ui::statusline::render_element(StatusElement::Diagnostics, &c.ed, &colors, "");
    assert!(
        text.is_empty(),
        "expected empty with no diagnostics, got {text:?}"
    );
}

#[test]
fn diagnostics_element_renders_error_and_warning_counts() {
    let c = setup(
        "abcdefgh\n",
        &[&[
            ((0, 0), (0, 1), 1),
            ((0, 2), (0, 3), 2),
            ((0, 4), (0, 5), 2),
        ]],
    );
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) =
        crate::ui::statusline::render_element(StatusElement::Diagnostics, &c.ed, &colors, "");
    assert_eq!(
        text.as_ref(),
        format!("{DIAGNOSTICS_ERROR_GLYPH} 1 {DIAGNOSTICS_WARNING_GLYPH} 2")
    );
}

#[test]
fn diagnostics_element_omits_zero_half() {
    let c = setup("abcdefgh\n", &[&[((0, 0), (0, 1), 1)]]);
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) =
        crate::ui::statusline::render_element(StatusElement::Diagnostics, &c.ed, &colors, "");
    assert_eq!(text.as_ref(), format!("{DIAGNOSTICS_ERROR_GLYPH} 1"));

    let c = setup("abcdefgh\n", &[&[((0, 0), (0, 1), 2)]]);
    let (text, _) =
        crate::ui::statusline::render_element(StatusElement::Diagnostics, &c.ed, &colors, "");
    assert_eq!(text.as_ref(), format!("{DIAGNOSTICS_WARNING_GLYPH} 1"));
}

#[test]
fn configure_statusline_round_trips_diagnostics_element_name() {
    let mut ed = Editor::open(None).unwrap();
    crate::editor::commands::typed_set(&mut ed, Some("global statusline=Diagnostics||"), false)
        .unwrap();
    assert_eq!(
        ed.state.settings.statusline.left,
        vec![StatusElement::Diagnostics]
    );
}

/// Tier 2: counts must track a second, corrected publish for the same file —
/// not just the first snapshot. Both publishes are queued before the single
/// `drain_lsp()` call (see `setup`'s doc comment).
#[test]
fn diagnostic_counts_update_across_a_corrected_publish() {
    let c = setup("abcdefgh\n", &[&[((0, 0), (0, 1), 1)], &[]]);
    let bid = c.ed.focused_buffer_id();
    assert_eq!(
        c.ed.diagnostic_counts(bid),
        (0, 0),
        "the corrected (empty) publish must replace the stale error count"
    );
}
