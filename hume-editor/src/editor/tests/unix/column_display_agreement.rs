// Column display agreement: `:diagnostics` and the LSP goto/references
// drawer must show the same grapheme column the statusline shows for the
// same position, not the char column or the raw UTF-16 wire column. See
// `hume-editor/src/ui/statusline/tests.rs`'s
// `position_element_shows_grapheme_column_not_char_or_utf16_count` for the
// statusline half and the independent-oracle derivation of this file's
// shared fixture line (grapheme col 2 / char col 3 / UTF-16 col 4 before
// 'x').
//
// Also covers `location_display_parts`'s per-location resolution: an open
// buffer gets an exact grapheme column; a location whose open buffer's line
// is out of range degrades to `#f` (a `path:line` row); a target with no
// open buffer renders its location's own wire `character` verbatim instead
// of reading the file to measure it — the one sanctioned exception to
// "never render a wire unit directly" (see `location_display_parts`'s doc,
// `hume-editor/src/editor/lsp/introspect.rs`). A location that can't be
// decoded at all (missing `range`) aborts the whole batch instead — see
// `a_malformed_location_aborts_the_batch_instead_of_a_degraded_row` below.
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// `"e\u{0301}\u{1D11E}x"` — 'e' + a combining acute accent (one grapheme
/// cluster, two `char`s, two UTF-16 units) then a musical-symbol astral
/// character (one grapheme cluster, one `char`, a surrogate pair = two
/// UTF-16 units), then 'x'. Counted by hand, not with any HUME helper:
/// before 'x', grapheme count = 2, char count = 3, UTF-16 code unit count
/// = 4 — three different values a wire `character: 4` must resolve to
/// display column 3 (grapheme), never 4 (char) or 5 (UTF-16 + 1).
const FIXTURE_LINE: &str = "e\u{0301}\u{1D11E}x\n";

/// All-ASCII line, used where the wire-column exception must produce the
/// *same* number as the exact grapheme column: byte offset, UTF-16 code
/// unit count, char count, and grapheme count all coincide once nothing
/// non-ASCII precedes the target position. `character: 4` names the space
/// right before `'x'` — grapheme/char/UTF-16 column 4 alike.
const ASCII_LINE: &str = "let x = 1;\n";

fn write_fixture_file(dir: &Path, name: &str) -> (PathBuf, String) {
    write_fixture_file_content(dir, name, FIXTURE_LINE)
}

fn write_fixture_file_content(dir: &Path, name: &str, content: &str) -> (PathBuf, String) {
    let file = dir.join(name);
    std::fs::write(&file, content).unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical)
        .unwrap()
        .as_str()
        .to_string();
    (file, uri)
}

// ── `:diagnostics` ───────────────────────────────────────────────────────

fn setup_diagnostics(file: &Path, tmp: &Path) -> (Editor, RealRuntimeGuard) {
    let guard = RealRuntimeGuard::new();
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    let uri = hume_lsp::uri::path_to_uri(file).unwrap();
    backend.push_from_server(
        sid,
        hume_lsp::codec::Message::Notification {
            method: "textDocument/publishDiagnostics".to_string(),
            params: serde_json::json!({"uri": uri.as_str(), "diagnostics": [{
                "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 5}},
                "severity": 1,
                "message": "astral column check",
            }]}),
        },
    );

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, file.parent().unwrap().to_path_buf()));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.drain_lsp();

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

/// Fail oracle: swap `diagnostics.scm`'s `:diagnostics` row back to reading
/// `"char-col"` — the row would show `1:4` (the char column) instead of the
/// correct grapheme column `1:3`.
#[test]
fn diagnostics_drawer_shows_grapheme_column() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path(), "main.rs");
    let (mut ed, _guard) = setup_diagnostics(&file, tmp.path());

    type_cmd(&mut ed, ":diagnostics");
    ed.settle();

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard.as_ref().expect("drawer must open").rows.clone()
    };
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].contains("1:3 "),
        "diagnostic anchored at 'x' (wire character 4) must show grapheme \
         column 3, not char column 4 or UTF-16 column 5, got {:?}",
        rows[0]
    );
}

// ── LSP references drawer ───────────────────────────────────────────────

fn setup_refs(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId) {
    let guard = RealRuntimeGuard::new();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();

    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"referencesProvider": true}}),
    );
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    configure(&mut backend, sid);
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

    (ed, guard, sid)
}

fn run_references(ed: &mut Editor) {
    // lsp-references is key-bindable, not typed — dispatch through the
    // keymap pipeline, the way its bound key (`z r`) would.
    ed.execute_keymap_command("lsp-references".into(), Some(1), false);
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

fn loc(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "uri": uri,
        "range": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character}}
    })
}

/// Nine locations, one `lsp-references` response, exercising every path in
/// `location_display_parts`: the focused buffer's own open rope, a second
/// open buffer that isn't the focused one, an unopened non-ASCII file (twice,
/// two different wire columns out of it, independent of each other), an
/// unopened file that doesn't exist on disk at all, a line past an open
/// buffer's end, that buffer's own phantom trailing line, an unopened target
/// whose URI needs percent-decoding, and an unopened all-ASCII file.
///
/// The central pin is rows 0 vs 1: the *same* wire location (`character: 4`
/// on the fixture's non-ASCII line) renders `1:3` from the open buffer
/// (measured grapheme column) and `1:5` from the unopened one (the raw wire
/// `character`, 1-based) — the divergence the wire-column exception accepts.
/// Rows 7 vs 8 are the mirror case: the same wire location on an all-ASCII
/// line renders the same number, `1:5`, whether the buffer is open or not,
/// because a byte offset, a UTF-16 code-unit count, and a grapheme count
/// coincide once nothing non-ASCII precedes the target — the reason the
/// exception is tolerable in practice.
///
/// Fail oracle: make the unopened branch call `wire_pos_to_grapheme_col`
/// against a freshly-read `BufferText` (this function's behavior before it stopped
/// reading files) — row 1 would read `1:3`, identical to row 0, and the
/// divergence assertion below would fail to catch a regression back to
/// reading the file.
#[test]
fn references_drawer_measures_open_buffers_and_echoes_wire_columns_for_unopened_targets() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path(), "main.rs");
    let (_other_file, other_uri) = write_fixture_file(file_dir.path(), "other.rs");
    // A space forces `path_to_uri` to percent-encode, so the URI and the
    // path it denotes are no longer the same string.
    let (_spaced_file, spaced_uri) = write_fixture_file(file_dir.path(), "a name.rs");
    assert!(
        spaced_uri.contains("%20"),
        "fixture must exercise percent-decoding, got {spaced_uri:?}"
    );
    let missing = file_dir.path().join("definitely_missing.rs");
    let missing_uri = format!("file://{}", missing.display());
    let (ascii_open_file, ascii_open_uri) =
        write_fixture_file_content(file_dir.path(), "ascii-open.rs", ASCII_LINE);
    let (_ascii_closed_file, ascii_closed_uri) =
        write_fixture_file_content(file_dir.path(), "ascii-closed.rs", ASCII_LINE);

    let (mut ed, _guard, _sid) = setup_refs(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([
                loc(&uri, 0, 4),              // open buffer, before 'x': displayed col 3
                loc(&other_uri, 0, 4),        // unopened, same position: displayed col 5
                loc(&other_uri, 0, 0),        // same unopened file again: displayed col 1
                loc(&missing_uri, 0, 0),      // unopened target that doesn't exist: displayed col 1
                loc(&uri, 5, 0),              // open buffer, past its one content line: no column
                loc(&uri, 1, 0),              // open buffer, its own phantom line: no column
                loc(&spaced_uri, 0, 4),       // percent-encoded uri: decoded path, displayed col 5
                loc(&ascii_open_uri, 0, 4),   // a second open buffer: displayed col 5
                loc(&ascii_closed_uri, 0, 4), // same text, unopened: displayed col 5 -- agrees
            ]),
        );
    });

    // A second buffer, open but not focused, so the open-buffer branch has
    // an all-ASCII line to measure. Refocuses `main.rs` afterward — `:e` on
    // an already-open path dedups onto the existing buffer rather than
    // reopening it, so `main.rs`'s attached LSP server (set below by
    // `setup_refs`) is unaffected and `:lsp-references` still dispatches
    // through it.
    ed.execute_typed("e", Some(ascii_open_file.to_str().unwrap()))
        .unwrap();
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();

    run_references(&mut ed);

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard.as_ref().expect("drawer must open").rows.clone()
    };
    assert_eq!(rows.len(), 9);
    assert!(
        rows[0].ends_with("main.rs:1:3"),
        "open-buffer location must show grapheme col 2 (1-based 3), got {:?}",
        rows[0]
    );
    assert!(
        rows[1].ends_with("other.rs:1:5"),
        "the identical position in an unopened file must show the wire \
         character verbatim (4, 1-based 5), diverging from the open \
         buffer's measured grapheme column, got {:?}",
        rows[1]
    );
    assert!(
        rows[2].ends_with("other.rs:1:1"),
        "a second location in the same unopened file must resolve \
         independently (wire char 0, 1-based 1), got {:?}",
        rows[2]
    );
    assert!(
        rows[3].ends_with("missing.rs:1:1"),
        "an unopened target that doesn't exist on disk still gets the wire \
         column verbatim -- this function no longer touches the disk to \
         find out, got {:?}",
        rows[3]
    );
    assert!(
        rows[4].ends_with("main.rs:6"),
        "an open buffer's line past its content must degrade to path:line, \
         no column, got {:?}",
        rows[4]
    );
    // The boundary the "past the end" case above is too far away to pin: the
    // fixture's one content line plus its structural `\n` make line 1 the
    // buffer's own phantom trailing line, which is a *valid ropey line* but
    // holds no content. Clamping to the ropey domain would resolve it to
    // grapheme column 0 and render `main.rs:2:1` — a row pointing one line
    // past the end of the file, with a column.
    assert!(
        rows[5].ends_with("main.rs:2"),
        "the phantom trailing line has no content, so it must degrade to \
         path:line with no column, got {:?}",
        rows[5]
    );
    // The row's path and its wire column must come from the same URI parse.
    // When Scheme rendered the path by stripping "file://" itself, it had no
    // percent-decoding: the row read "a%20name.rs" while the column beside
    // it had been read out of "a name.rs" — one row naming a file it did
    // not measure.
    assert!(
        rows[6].ends_with("a name.rs:1:5"),
        "a percent-encoded uri must render its decoded path alongside the \
         wire column read from that same location, got {:?}",
        rows[6]
    );
    assert!(
        rows[7].ends_with("ascii-open.rs:1:5"),
        "a second open buffer (not the focused one) must still get an \
         exact grapheme column, got {:?}",
        rows[7]
    );
    assert!(
        rows[8].ends_with("ascii-closed.rs:1:5"),
        "the identical position in the same text, unopened, must show the \
         same number as row 7 -- on an all-ASCII line the wire column and \
         the grapheme column coincide, got {:?}",
        rows[8]
    );
}

/// A location missing `range` entirely names no destination `goto-location!`
/// could jump to either, so `lsp-locations->display-parts` must abort the
/// whole batch rather than render an unselectable row for it — see
/// `hume_lsp::location::decode_location`'s doc.
///
/// Sabotage oracle: loosen `decode_location` to tolerate a missing `range`
/// (e.g. defaulting to line 0) — the drawer would open with four rows
/// instead of erroring, and this test would fail.
#[test]
fn a_malformed_location_aborts_the_batch_instead_of_a_degraded_row() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path(), "main.rs");

    let (mut ed, _guard, _sid) = setup_refs(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([
                loc(&uri, 0, 0),
                loc(&uri, 0, 4),
                loc(&uri, 0, 0),
                {"uri": uri}, // missing `range` entirely
            ]),
        );
    });

    run_references(&mut ed);

    assert!(
        ed.state.drawer_view.read().unwrap().is_none(),
        "a malformed location must abort before the drawer ever opens, \
         not open it with the good rows and drop the bad one"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("lsp-locations->display-parts") && msg.contains("missing range"),
        "expected an error naming the builtin and the missing field, got {msg:?}"
    );
}
