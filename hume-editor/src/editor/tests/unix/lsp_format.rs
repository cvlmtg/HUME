// Formatting: `:lsp-fmt`, composing `lsp-request`,
// `lsp-capabilities`, `selections-linewise?`/`selections-charwise?`, `apply-text-edits!`.
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_editing::selection::{Selection, SelectionSet};
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// Every test's buffer content unless a test needs a different line shape.
/// Char offsets: line0 'line1' = 0..5 (+\n at 5), line1 'line2' = 6..11
/// (+\n at 11), line2 'line3' = 12..17 (+\n at 17) — the selection helpers
/// below reference these offsets directly.
const THREE_LINES: &str = "line1\nline2\nline3\n";

/// Handshake caps advertise `rangeFormatting` without `rangesSupport` — the
/// common case, and what every fan-out test wants.
fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard) {
    setup_with_content(file, tmp, THREE_LINES, configure)
}

/// Same as `setup`, with the file content under caller control — for a test
/// needing a different line shape (e.g. a blank line).
fn setup_with_content(
    file: &Path,
    tmp: &Path,
    content: &str,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard) {
    setup_with_caps(
        file,
        tmp,
        content,
        serde_json::json!({"capabilities": {
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": true
        }}),
        configure,
    )
}

/// Same as `setup_with_content`, with the handshake's `initialize` result
/// also under caller control — for the `rangesSupport` tests, which need it
/// to differ from the common case above.
fn setup_with_caps(
    file: &Path,
    tmp: &Path,
    content: &str,
    initialize_result: serde_json::Value,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard) {
    let guard = RealRuntimeGuard::new();
    std::fs::write(file, content).unwrap();

    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", initialize_result);
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    configure(&mut backend, sid);

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(load-plugin "core:stdlib")
(load-plugin "core:lsp")"#,
        tmp,
    );
    ed.scripting = Some(host);

    (ed, guard)
}

fn select_full_line_1(ed: &mut Editor) {
    // 'line1\n' — chars [0, 6).
    ed.set_current_selections(SelectionSet::single(Selection::new(0, 5)));
}

fn select_full_lines_1_and_3(ed: &mut Editor) {
    // Two disjoint linewise selections: 'line1\n' (chars [0, 6)) and
    // 'line3\n' (chars [12, 17]) — 'line2\n' in between is untouched by
    // either.
    ed.set_current_selections(SelectionSet::from_vec(
        vec![Selection::new(0, 5), Selection::new(12, 17)],
        0,
    ));
}

fn select_full_line_1_and_a_sub_line_selection(ed: &mut Editor) {
    // 'line1\n' whole (chars [0, 6)), plus "lin" on line 2 (chars 6..=8) —
    // not linewise.
    ed.set_current_selections(SelectionSet::from_vec(
        vec![Selection::new(0, 5), Selection::new(6, 8)],
        0,
    ));
}

/// For `"line1\n\nline3\n"` (a blank line2): a real charwise selection on
/// "in" within line1 (chars 1..=2), plus a collapsed cursor on the blank
/// line2 (char 6) — the shape a multi-cursor command can leave behind when
/// one cursor happens to land on a blank line.
fn select_mid_line_and_a_blank_line_cursor(ed: &mut Editor) {
    ed.set_current_selections(SelectionSet::from_vec(
        vec![Selection::new(1, 2), Selection::collapsed(6)],
        0,
    ));
}

fn run_fmt(ed: &mut Editor) {
    type_cmd(ed, ":lsp-fmt");
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
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
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
        },
    );
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

#[test]
fn sub_line_selection_still_formats_the_whole_buffer() {
    // Default cursor: a bare collapsed selection — never spans a full line.
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
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
        },
    );

    run_fmt(&mut ed);

    assert_eq!(ed.doc().text().to_string(), "WHOLE_BUFFER\n");
}

#[test]
fn full_line_selection_sends_range_formatting() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "RANGE_FORMATTED\n")]),
            );
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
            );
        },
    );
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
/// covering the untouched line in between — `:lsp-fmt` sends one
/// `rangeFormatting` per range instead (the server here doesn't advertise
/// `rangesSupport`), and applies both edits as a single transaction.
#[test]
fn disjoint_full_line_selections_send_two_range_formatting_requests() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
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
        },
    );
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE1\nline2\nRANGE3\n",
        "a gap between two linewise selections must send one rangeFormatting \
         per range and apply both edits, not fall back to the whole buffer"
    );
}

/// Same shape as above, but the server advertises `rangesSupport` — one
/// `textDocument/rangesFormatting` request carrying both ranges, not two
/// separate `rangeFormatting` round trips.
#[test]
fn disjoint_full_line_selections_send_one_ranges_formatting_request_when_supported() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup_with_caps(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        THREE_LINES,
        serde_json::json!({"capabilities": {
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": {"rangesSupport": true}
        }}),
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
         carrying both ranges — a fan-out or whole-buffer request instead would \
         either apply a decoy edit or (fan-out, with only one decoy queued) leave \
         the buffer unchanged, neither of which matches"
    );
}

/// A single full-line selection stays on the `rangeFormatting` path even
/// when the server advertises `rangesSupport` — that capability only
/// changes how *multiple* ranges are sent (one `rangesFormatting` request
/// instead of a fan-out), and one range has nothing to batch with.
#[test]
fn single_full_line_selection_sends_range_formatting_even_when_ranges_supported() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup_with_caps(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        THREE_LINES,
        serde_json::json!({"capabilities": {
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": {"rangesSupport": true}
        }}),
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
/// but past `lsp.format-max-ranges` — a server advertising `rangesSupport` sends every
/// range in one `rangesFormatting` request, so the cap (which bounds one-request-per-
/// range fan-out) has nothing to bound and never fires.
#[test]
fn ranges_formatting_is_not_capped_by_format_max_ranges() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup_with_caps(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        THREE_LINES,
        serde_json::json!({"capabilities": {
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": {"rangesSupport": true}
        }}),
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
    type_cmd(&mut ed, ":set global lsp.format-max-ranges=1");
    select_full_lines_1_and_3(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE1\nline2\nRANGE3\n",
        "a rangesSupport server must format both ranges in one request even when \
         their count exceeds lsp.format-max-ranges — the cap only bounds the \
         one-request-per-range fan-out path"
    );
}

/// A buffer with no attached server reports that, and sends no request — the
/// coverage `lsp/guard-capability` used to give this case before
/// `lsp-linewise-ranges-params` (which returns `#f` for it) replaced the
/// direct capability check.
#[test]
fn no_attached_server_reports_and_sends_nothing() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            // Decoy proving no request at all is sent.
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
            );
        },
    );
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = None;
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
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            // Decoy proving no request at all is sent.
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
            );
        },
    );
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

/// A whole-line selection and a sub-line selection together are ambiguous
/// — `:lsp-fmt` warns and formats nothing, rather than guessing which
/// reading the user meant.
#[test]
fn mixed_linewise_and_sub_line_selections_warn_and_format_nothing() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            // Decoys proving no request at all is sent for a mixed set.
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
            );
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "WRONG_RANGE_PATH\n")]),
            );
        },
    );
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
/// not a deliberate whole-line selection — end-to-end through `:lsp-fmt`,
/// on top of the unit coverage in `buffer_text_steel.rs` and
/// `lsp_introspect.rs`.
#[test]
fn stray_blank_line_cursor_does_not_trigger_the_mixed_selection_refusal() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup_with_content(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        "line1\n\nline3\n",
        |backend, _sid| {
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
        },
    );
    select_mid_line_and_a_blank_line_cursor(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "WHOLE_BUFFER\n",
        "a real charwise selection plus a stray blank-line cursor must whole-buffer \
         format, not warn about a mixed selection set"
    );
}

/// Past `lsp.format-max-ranges`, `:lsp-fmt` refuses and warns instead of
/// silently narrowing an N-region request into a 1-region one — the same
/// contract the mixed-selection case above uses for "can't do this
/// unambiguously".
#[test]
fn fan_out_past_the_cap_warns_and_formats_nothing() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            // Decoy proving no request at all is sent past the cap.
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "WRONG_PRIMARY_ONLY_PATH\n")]),
            );
        },
    );
    type_cmd(&mut ed, ":set global lsp.format-max-ranges=1");
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
        msg.to_lowercase().contains("lsp.format-max-ranges") && msg.contains('1'),
        "expected a warning naming the setting and its value, got {msg:?}"
    );
}

/// The cap is exclusive — exactly `lsp.format-max-ranges` ranges still fans
/// out normally, only `n > cap` refuses.
#[test]
fn fan_out_at_the_cap_formats_normally() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
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
    type_cmd(&mut ed, ":set global lsp.format-max-ranges=2");
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
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
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

/// One range's error response aborts the whole fan-out — no partial
/// format from the range that did succeed.
#[test]
fn fan_out_error_response_applies_nothing() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "RANGE1\n")]),
            );
            backend.fail_with("textDocument/rangeFormatting", -32603, "boom");
        },
    );
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
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            backend.respond_to("textDocument/formatting", serde_json::Value::Null);
        },
    );

    run_fmt(&mut ed);

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("already formatted"),
        "expected an already-formatted message, got {msg:?}"
    );
}

#[test]
fn loading_the_plugin_registers_no_save_hook() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            // If a save hook incorrectly fired :lsp-fmt, this response landing
            // would visibly rewrite the buffer.
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "SHOULD_NOT_APPEAR\n")]),
            );
        },
    );
    let before = ed.doc().text().to_string();

    let bid = ed.focused_buffer_id();
    ed.queue_buffer_save(bid);
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "loading core:lsp must not register an on-buffer-save formatter — v1 is manual :lsp-fmt only"
    );
}
