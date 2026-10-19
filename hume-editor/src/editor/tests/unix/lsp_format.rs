// Formatting: `:format-source`, composing `lsp-request!`,
// `lsp-capabilities`, `selections-linewise?`/`selections-charwise?`, `apply-text-edits!`.
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeDirs`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use crate::editor::tests::lsp_rig::{LspRig, stop_server};
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;

/// Every test's buffer content unless a test needs a different line shape.
/// Char offsets: line0 'line1' = 0..5 (+\n at 5), line1 'line2' = 6..11
/// (+\n at 11), line2 'line3' = 12..17 (+\n at 17). The selection helpers
/// below reference these offsets directly.
const THREE_LINES: &str = "line1\nline2\nline3\n";

/// Handshake caps advertise `rangeFormatting` without `rangesSupport`: the
/// common case, and what every fan-out test wants.
fn range_caps() -> serde_json::Value {
    serde_json::json!({"capabilities": {
        "documentFormattingProvider": true,
        "documentRangeFormattingProvider": true
    }})
}

fn setup(
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs) {
    setup_configured(tmp, None, configure)
}

/// Same as `setup`, with `config` passed to `core:lsp` as its `#:config`.
fn setup_configured(
    tmp: &Path,
    config: Option<&str>,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs) {
    setup_with_caps(tmp, THREE_LINES, range_caps(), config, configure)
}

/// Same as `setup`, with the file content under caller control, for a test
/// needing a different line shape (e.g. a blank line).
fn setup_with_content(
    tmp: &Path,
    content: &str,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs) {
    setup_with_caps(tmp, content, range_caps(), None, configure)
}

/// Same as `setup_configured`, with the content and the handshake's
/// `initialize` result also under caller control.
fn setup_with_caps(
    tmp: &Path,
    content: &str,
    initialize_result: serde_json::Value,
    config: Option<&str>,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs) {
    let (rig, guard) = core_lsp_rig_configured(
        tmp,
        &marked_at_start(content),
        initialize_result,
        config,
        configure,
    );
    (rig.ed, guard)
}

fn select_full_line_1(ed: &mut Editor) {
    // 'line1\n': chars [0, 6).
    select(ed, &[(0, 5)], 0);
}

fn select_full_lines_1_and_3(ed: &mut Editor) {
    // Two disjoint linewise selections: 'line1\n' (chars [0, 6)) and
    // 'line3\n' (chars [12, 17]). 'line2\n' in between is untouched by
    // either.
    select(ed, &[(0, 5), (12, 17)], 0);
}

fn select_full_line_1_and_a_sub_line_selection(ed: &mut Editor) {
    // 'line1\n' whole (chars [0, 6)), plus "lin" on line 2 (chars 6..=8),
    // not linewise.
    select(ed, &[(0, 5), (6, 8)], 0);
}

/// For `"line1\n\nline3\n"` (a blank line2): a real charwise selection on
/// "in" within line1 (chars 1..=2), plus a collapsed cursor on the blank
/// line2 (char 6): the shape a multi-cursor command can leave behind when
/// one cursor happens to land on a blank line.
fn select_mid_line_and_a_blank_line_cursor(ed: &mut Editor) {
    select(ed, &[(1, 2), (6, 6)], 0);
}

fn run_fmt(ed: &mut Editor) {
    // :format-source and the key-bindable lsp/format-source share one body
    // (lsp/format-source!); dispatch through the typed `:` entry point.
    type_cmd(ed, ":format-source");
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

fn text_edit(sl: u64, sc: u64, el: u64, ec: u64, new_text: &str) -> serde_json::Value {
    serde_json::json!({
        "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
        "newText": new_text
    })
}

#[test]
fn whole_buffer_edit_is_one_undo_step() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(
                0,
                0,
                3,
                0,
                "formatted1\nformatted2\nformatted3\n"
            )]),
        );
    });
    let before = ed.doc().text().to_string();

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "formatted1\nformatted2\nformatted3\n",
        "the whole-buffer replacement edit must apply"
    );
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "a single 'u' must fully restore the pre-format text"
    );
}

/// `:format-source` (typed) and `lsp/format-source` (editor command, dispatched here
/// via `call!` the way an `on-buffer-save` hook does) share one
/// implementation (`lsp/format-source!`), so both must format identically.
#[test]
fn format_source_and_lsp_fmt_call_produce_the_same_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(
                0,
                0,
                3,
                0,
                "formatted1\nformatted2\nformatted3\n"
            )]),
        );
    });

    // lsp/format-source is key-bindable, not typed: dispatch through the keymap
    // pipeline, the way an `on-buffer-save` hook's `(call! "lsp/format-source" bid)`
    // would.
    ed.execute_keymap_command("lsp/format-source".into(), Some(1), false);
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        "formatted1\nformatted2\nformatted3\n",
        "lsp/format-source (editor command) must format the same way :format-source does"
    );
}

#[test]
fn sub_line_selection_still_formats_the_whole_buffer() {
    // Default cursor: a bare collapsed selection, never spans a full line.
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "WHOLE_BUFFER\n")]),
        );
        // If the decision were wrong and a sub-line selection triggered
        // range formatting instead, this response would apply and the
        // assertion below would fail loudly (not silently match).
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "WRONG_RANGE_PATH\n")]),
        );
    });

    run_fmt(&mut ed);

    assert_eq!(ed.doc().text().to_string(), "WHOLE_BUFFER\n");
}

#[test]
fn full_line_selection_sends_range_formatting() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "RANGE_FORMATTED\n")]),
        );
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
        );
    });
    select_full_line_1(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE_FORMATTED\nline2\nline3\n",
        "a full-line selection must send rangeFormatting, not whole-buffer formatting"
    );
}

/// Two disjoint linewise selections (line 1 and line 3, with line 2
/// untouched by either) can't be expressed as one LSP range without also
/// covering the untouched line in between, so `:format-source` sends one
/// `rangeFormatting` per range instead (the server here doesn't advertise
/// `rangesSupport`), and applies both edits as a single transaction.
#[test]
fn disjoint_full_line_selections_send_two_range_formatting_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        // FIFO per method: line 1's request gets this one first...
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "RANGE1\n")]),
        );
        // ...line 3's request gets this one second.
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(2, 0, 3, 0, "RANGE3\n")]),
        );
        // If the decision were wrong and this sent one whole-buffer
        // request instead, this response would apply and the assertion
        // below would fail loudly.
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
        );
    });
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE1\nline2\nRANGE3\n",
        "a gap between two linewise selections must send one rangeFormatting \
         per range and apply both edits, not fall back to the whole buffer"
    );
}

/// Same shape as above, but the server advertises `rangesSupport`: one
/// `textDocument/rangesFormatting` request carrying both ranges, not two
/// separate `rangeFormatting` round trips.
#[test]
fn disjoint_full_line_selections_send_one_ranges_formatting_request_when_supported() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup_with_caps(
        tmp.path(),
        THREE_LINES,
        serde_json::json!({"capabilities": {
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": {"rangesSupport": true}
        }}),
        None,
        |backend, _sid| {
            backend.respond_to(
                "textDocument/rangesFormatting",
                serde_json::json!([
                    text_edit(0, 0, 1, 0, "RANGE1\n"),
                    text_edit(2, 0, 3, 0, "RANGE3\n")
                ]),
            );
            // Decoys proving neither the whole-buffer nor the per-range
            // fan-out path was taken instead.
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
            );
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_FAN_OUT_PATH\n")]),
            );
        },
    );
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE1\nline2\nRANGE3\n",
        "a server advertising rangesSupport must get one rangesFormatting request \
         carrying both ranges; a fan-out or whole-buffer request instead would \
         either apply a decoy edit or (fan-out, with only one decoy queued) leave \
         the buffer unchanged, neither of which matches"
    );
}

/// A single full-line selection stays on the `rangeFormatting` path even
/// when the server advertises `rangesSupport`. That capability only
/// changes how *multiple* ranges are sent (one `rangesFormatting` request
/// instead of a fan-out), and one range has nothing to batch with.
#[test]
fn single_full_line_selection_sends_range_formatting_even_when_ranges_supported() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup_with_caps(
        tmp.path(),
        THREE_LINES,
        serde_json::json!({"capabilities": {
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": {"rangesSupport": true}
        }}),
        None,
        |backend, _sid| {
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "RANGE_FORMATTED\n")]),
            );
            // Decoys proving neither the whole-buffer nor the
            // rangesFormatting path was taken instead.
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
            );
            backend.respond_to(
                "textDocument/rangesFormatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_RANGES_PATH\n")]),
            );
        },
    );
    select_full_line_1(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE_FORMATTED\nline2\nline3\n",
        "a single linewise selection must send rangeFormatting, not \
         rangesFormatting or whole-buffer formatting, even when the server \
         advertises rangesSupport"
    );
}

/// Same shape as `disjoint_full_line_selections_send_two_range_formatting_requests`,
/// but past `format-max-ranges`: a server advertising `rangesSupport` sends every
/// range in one `rangesFormatting` request, so the cap (which bounds one-request-per-
/// range fan-out) has nothing to bound and never fires.
#[test]
fn ranges_formatting_is_not_capped_by_format_max_ranges() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup_with_caps(
        tmp.path(),
        THREE_LINES,
        serde_json::json!({"capabilities": {
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": {"rangesSupport": true}
        }}),
        Some(r#"(hash "format-max-ranges" 1)"#),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/rangesFormatting",
                serde_json::json!([
                    text_edit(0, 0, 1, 0, "RANGE1\n"),
                    text_edit(2, 0, 3, 0, "RANGE3\n")
                ]),
            );
        },
    );
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE1\nline2\nRANGE3\n",
        "a rangesSupport server must format both ranges in one request even when \
         their count exceeds format-max-ranges; the cap only bounds the \
         one-request-per-range fan-out path"
    );
}

/// A buffer with no attached server reports that, and sends no request.
/// `lsp-linewise-ranges-params` returns `#f` for it.
#[test]
fn no_attached_server_reports_and_sends_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        // Decoy proving no request at all is sent.
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
        );
    });
    stop_server(&mut ed, "rust-analyzer");
    let before = ed.doc().text().to_string();

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "no attached server must format nothing"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no lsp server attached"),
        "expected a no-server warning, got {msg:?}"
    );
}

/// A buffer with an attached server but no path (e.g. a scratch buffer)
/// reports that distinctly from the no-server case above, and sends no
/// request.
#[test]
fn buffer_with_no_path_reports_and_sends_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        // Decoy proving no request at all is sent.
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
        );
    });
    ed.doc_mut().set_path(None);
    let before = ed.doc().text().to_string();

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "a pathless buffer must format nothing"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no path"),
        "expected a no-path warning, got {msg:?}"
    );
}

/// A whole-line selection and a sub-line selection together are ambiguous:
/// `:format-source` warns and formats nothing, rather than guessing which
/// reading the user meant.
#[test]
fn mixed_linewise_and_sub_line_selections_warn_and_format_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        // Decoys proving no request at all is sent for a mixed set.
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
        );
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "WRONG_RANGE_PATH\n")]),
        );
    });
    let before = ed.doc().text().to_string();
    select_full_line_1_and_a_sub_line_selection(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "a mixed linewise/partial selection set must format nothing"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("mixed"),
        "expected a warning naming the mixed selection, got {msg:?}"
    );
}

/// A stray blank-line cursor is ambiguous (see `linewise_classification`),
/// not a deliberate whole-line selection. End-to-end through `:format-source`,
/// on top of the unit coverage in `buffer_text_steel.rs` and
/// `lsp_introspect.rs`.
#[test]
fn stray_blank_line_cursor_does_not_trigger_the_mixed_selection_refusal() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup_with_content(tmp.path(), "line1\n\nline3\n", |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "WHOLE_BUFFER\n")]),
        );
        // Decoy proving the range path (mixed refusal's usual
        // companion) was not taken instead.
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "WRONG_RANGE_PATH\n")]),
        );
    });
    select_mid_line_and_a_blank_line_cursor(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "WHOLE_BUFFER\n",
        "a real charwise selection plus a stray blank-line cursor must whole-buffer \
         format, not warn about a mixed selection set"
    );
}

/// Past `format-max-ranges`, `:format-source` refuses and warns instead of
/// silently narrowing an N-region request into a 1-region one: the same
/// contract the mixed-selection case above uses for "can't do this
/// unambiguously".
#[test]
fn fan_out_past_the_cap_warns_and_formats_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup_configured(
        tmp.path(),
        Some(r#"(hash "format-max-ranges" 1)"#),
        |backend, _sid| {
            // Decoy proving no request at all is sent past the cap.
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "WRONG_PRIMARY_ONLY_PATH\n")]),
            );
        },
    );
    let before = ed.doc().text().to_string();
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "exceeding the cap must format nothing, not narrow to the primary selection"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("format-max-ranges") && msg.contains('1'),
        "expected a warning naming the config key and its value, got {msg:?}"
    );
}

/// The cap is exclusive: a range count equal to `format-max-ranges` still
/// fans out normally, only `n > cap` refuses.
#[test]
fn fan_out_at_the_cap_formats_normally() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup_configured(
        tmp.path(),
        Some(r#"(hash "format-max-ranges" 2)"#),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "RANGE1\n")]),
            );
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(2, 0, 3, 0, "RANGE3\n")]),
            );
        },
    );
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE1\nline2\nRANGE3\n",
        "a range count equal to the cap must still format, not refuse"
    );
}

/// The fan-out's edits land as one undo step, same as the whole-buffer and
/// single-range paths.
#[test]
fn fan_out_applies_as_one_undo_step() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "RANGE1\n")]),
        );
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(2, 0, 3, 0, "RANGE3\n")]),
        );
    });
    let before = ed.doc().text().to_string();
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "RANGE1\nline2\nRANGE3\n");

    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "a single 'u' must fully restore the pre-format text"
    );
}

/// One range's error response aborts the whole fan-out, with no partial
/// format from the range that did succeed.
#[test]
fn fan_out_error_response_applies_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "RANGE1\n")]),
        );
        backend.fail_with("textDocument/rangeFormatting", -32603, "boom");
    });
    let before = ed.doc().text().to_string();
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "an error on any one range must abort the whole fan-out with no partial edit"
    );
}

#[test]
fn null_result_reports_already_formatted() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/formatting", serde_json::Value::Null);
    });

    run_fmt(&mut ed);

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("already formatted"),
        "expected an already-formatted message, got {msg:?}"
    );
}

#[test]
fn format_reply_applies_after_the_pane_switches_buffers() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, FORMATTED)]),
        );
    });
    let formatted_bid = ed.focused_buffer_id();

    type_cmd(&mut ed, ":format-source");
    let (other, _other_path) = file_buffer("world\n");
    let other_bid = ed.open_buffer(other);
    let pid = ed.view.focused_pane();
    crate::editor::focus::switch_pane_to_buffer(&mut ed.state, &mut ed.view, pid, other_bid);
    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(formatted_bid).text().to_string(),
        FORMATTED
    );
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "the reply must apply without an error"
    );
}

#[test]
fn loading_the_plugin_registers_no_save_hook() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard) = setup(tmp.path(), |backend, _sid| {
        // If a save hook incorrectly fired :format-source, this response landing
        // would visibly rewrite the buffer.
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "SHOULD_NOT_APPEAR\n")]),
        );
    });
    let before = ed.doc().text().to_string();

    let bid = ed.focused_buffer_id();
    ed.queue_buffer_save(bid);
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "loading core:lsp must not register an on-buffer-save formatter: v1 is manual :format-source only"
    );
}

// ── lsp/format-document, as format-on-save drives it ────────────────────────────

const FORMAT_ON_SAVE: &str = r#"(register-hook! 'on-before-buffer-save
  (lambda (pane done) (call! "lsp/format-document" pane done)))"#;

const FORMATTED: &str = "formatted1\nformatted2\nformatted3\n";

fn save_rig(
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (LspRig, RealRuntimeDirs) {
    let (mut rig, guard) = core_lsp_rig_configured(
        tmp,
        &marked_at_start(THREE_LINES),
        range_caps(),
        None,
        configure,
    );
    rig.eval(FORMAT_ON_SAVE);
    (rig, guard)
}

fn on_disk(rig: &LspRig) -> String {
    std::fs::read_to_string(rig.root.join("src/main.rs")).unwrap()
}

fn save(rig: &mut LspRig) {
    type_cmd(&mut rig.ed, ":w");
    rig.ed.settle();
    rig.ed.drain_lsp();
    rig.ed.settle();
}

#[test]
fn format_on_save_writes_the_formatted_text() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, FORMATTED)]),
        );
    });
    assert_eq!(on_disk(&rig), THREE_LINES);

    save(&mut rig);

    assert_eq!(on_disk(&rig), FORMATTED);
    assert!(!rig.ed.state.has_unsaved_changes(rig.bid));
}

#[test]
fn format_on_save_formats_the_whole_document_whatever_is_selected() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, FORMATTED)]),
        );
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "WRONG_RANGE_PATH\n")]),
        );
    });
    select_full_line_1(&mut rig.ed);

    save(&mut rig);

    assert_eq!(on_disk(&rig), FORMATTED);
}

#[test]
fn format_on_save_writes_unformatted_text_when_the_server_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |backend, _sid| {
        backend.fail_with("textDocument/formatting", -32603, "boom");
    });
    rig.ed.handle_key(key('i'));
    rig.ed.handle_key(key('x'));
    rig.ed.handle_key(key_esc());

    save(&mut rig);

    assert_eq!(on_disk(&rig), format!("x{THREE_LINES}"));
}

#[test]
fn format_on_save_writes_when_no_server_is_attached() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |_backend, _sid| {});
    stop_server(&mut rig.ed, "rust-analyzer");
    rig.ed.handle_key(key('i'));
    rig.ed.handle_key(key('x'));
    rig.ed.handle_key(key_esc());

    save(&mut rig);

    assert_eq!(on_disk(&rig), format!("x{THREE_LINES}"));
}

#[test]
fn format_on_save_with_no_ranges_finishes_without_waiting_for_the_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = core_lsp_rig_configured(
        tmp.path(),
        &marked_at_start(THREE_LINES),
        range_caps(),
        None,
        |_backend, _sid| {},
    );
    rig.eval(
        r#"(register-hook! 'on-before-buffer-save
             (lambda (pane done) (call! "lsp/format-document" pane done (list))))"#,
    );
    type_cmd(&mut rig.ed, ":set global hook-before-save-timeout-ms=60000");
    rig.ed.handle_key(key('i'));
    rig.ed.handle_key(key('x'));
    rig.ed.handle_key(key_esc());

    save(&mut rig);

    assert_eq!(on_disk(&rig), format!("x{THREE_LINES}"));
}

#[test]
fn lsp_fmt_document_without_a_server_reports_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |_backend, _sid| {});
    stop_server(&mut rig.ed, "rust-analyzer");
    rig.eval(
        r#"(define-typed-command! "fmt-doc" ""
             (lambda (pane) (call! "lsp/format-document" pane #f)))"#,
    );

    type_cmd(&mut rig.ed, ":fmt-doc");
    rig.ed.settle();
    rig.ed.drain_lsp();
    rig.ed.settle();

    let status = rig.ed.state.status_msg.clone().unwrap_or_default();
    assert!(!status.contains("lsp"), "unexpected report: {status:?}");
}

#[test]
fn lsp_fmt_document_formats_only_the_ranges_it_is_given() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/rangeFormatting",
            serde_json::json!([text_edit(0, 0, 1, 0, "RANGE_FORMATTED\n")]),
        );
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
        );
    });
    rig.eval(
        r#"(define-typed-command! "fmt-ranges" ""
             (lambda (pane)
               (call! "lsp/format-document" pane #f
                      (hash-ref (lsp-linewise-ranges-params pane) "ranges"))))"#,
    );
    select_full_line_1(&mut rig.ed);

    type_cmd(&mut rig.ed, ":fmt-ranges");
    rig.ed.settle();
    rig.ed.drain_lsp();
    rig.ed.settle();

    assert_eq!(
        rig.ed.doc().text().to_string(),
        "RANGE_FORMATTED\nline2\nline3\n"
    );
}

#[test]
fn format_on_save_formats_a_buffer_no_pane_shows() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, FORMATTED)]),
        );
    });
    rig.ed.handle_key(key('i'));
    rig.ed.handle_key(key('x'));
    rig.ed.handle_key(key_esc());
    type_cmd(
        &mut rig.ed,
        &format!(":e {}", rig.root.join("other.txt").display()),
    );
    assert_ne!(
        rig.ed.focused_buffer_id(),
        rig.bid,
        "no pane may show the buffer"
    );

    type_cmd(&mut rig.ed, ":wa");
    rig.ed.settle();
    rig.ed.drain_lsp();
    rig.ed.settle();

    assert_eq!(on_disk(&rig), FORMATTED);
}

#[test]
fn a_format_reply_after_the_timeout_formats_the_saved_buffer() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, FORMATTED)]),
        );
    });
    type_cmd(&mut rig.ed, ":set global hook-before-save-timeout-ms=1");
    rig.ed.handle_key(key('i'));
    rig.ed.handle_key(key('x'));
    rig.ed.handle_key(key_esc());

    type_cmd(&mut rig.ed, ":w");
    rig.ed.settle();
    std::thread::sleep(std::time::Duration::from_millis(30));
    rig.ed.settle();
    assert_eq!(on_disk(&rig), format!("x{THREE_LINES}"));

    rig.ed.drain_lsp();
    rig.ed.settle();

    assert_eq!(rig.ed.doc().text().to_string(), FORMATTED);
    assert!(rig.ed.state.has_unsaved_changes(rig.bid));
}

#[test]
fn undo_after_format_on_save_puts_the_requesting_pane_cursor_back() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = save_rig(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/formatting",
            serde_json::json!([text_edit(0, 0, 3, 0, FORMATTED)]),
        );
    });
    rig.ed.handle_key(key('j'));
    let head = rig.ed.current_view().primary().head().offset();
    assert_ne!(head, hume_rope::offset::CharOffset::default());

    save(&mut rig);
    assert_eq!(on_disk(&rig), FORMATTED);
    rig.ed.handle_key(key('u'));

    assert_eq!(rig.ed.doc().text().to_string(), THREE_LINES);
    assert_eq!(rig.ed.current_view().primary().head().offset(), head);
}
