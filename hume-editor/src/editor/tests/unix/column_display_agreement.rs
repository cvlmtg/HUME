// Column display agreement: `:diagnostics` and the LSP goto/references
// drawer must show the same grapheme column the statusline shows for the
// same position, not the char column or the raw UTF-16 wire column. See
// `hume-editor/src/ui/statusline/tests.rs`'s
// `position_element_shows_grapheme_column_not_char_or_utf16_count` for the
// statusline half and the independent-oracle derivation of this file's
// shared fixture line (grapheme col 2 / char col 3 / UTF-16 col 4 before
// 'x'). Also covers `location_grapheme_cols`'s per-location resolution:
// open buffer, unopened file read from disk (twice, same file, one read),
// missing file, and an out-of-range line — each degrades to `#f` (a
// `path:line` row) rather than a wrong number.
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

fn write_fixture_file(dir: &Path, name: &str) -> (PathBuf, String) {
    let file = dir.join(name);
    std::fs::write(&file, FIXTURE_LINE).unwrap();
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
    eval_with_real_host(&mut ed, &mut host, r#"(load-plugin "core:lsp")"#, tmp);
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
    eval_with_real_host(&mut ed, &mut host, r#"(load-plugin "core:lsp")"#, tmp);
    ed.scripting = Some(host);

    (ed, guard, sid)
}

fn run_references(ed: &mut Editor) {
    type_cmd(ed, ":lsp-references");
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

/// Seven locations, one `lsp-references` response, exercising every path in
/// `location_display_parts`: the focused buffer's own open rope, the same
/// unopened file read from disk twice (one cached read, two different
/// grapheme columns out of it), a target file that doesn't exist, a line
/// past the target's end, the buffer's own phantom trailing line, and a
/// target whose URI needs percent-decoding.
///
/// Fail oracle: revert `lsp/location-display` to render the raw wire
/// `character` (this range's prior behavior) — row 0 would read `1:5`
/// (character 4, 1-based) instead of the correct `1:3`.
#[test]
fn references_drawer_shows_grapheme_columns_across_open_and_disk_files() {
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

    let (mut ed, _guard, _sid) = setup_refs(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([
                loc(&uri, 0, 4),         // open buffer: 'x' -> grapheme col 3
                loc(&other_uri, 0, 4),   // unopened disk file: same position
                loc(&other_uri, 0, 0),   // same unopened file again: 'e' -> grapheme col 1
                loc(&missing_uri, 0, 0), // unreadable target: no column
                loc(&uri, 5, 0),         // past the file's one content line: no column
                loc(&uri, 1, 0),         // the buffer's own phantom line: no column
                loc(&spaced_uri, 0, 4),  // percent-encoded uri: decoded path + col 3
            ]),
        );
    });

    run_references(&mut ed);

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard.as_ref().expect("drawer must open").rows.clone()
    };
    assert_eq!(rows.len(), 7);
    assert!(
        rows[0].ends_with("main.rs:1:3"),
        "open-buffer location must show grapheme col 3, got {:?}",
        rows[0]
    );
    assert!(
        rows[1].ends_with("other.rs:1:3"),
        "unopened-file location must show grapheme col 3 too, got {:?}",
        rows[1]
    );
    assert!(
        rows[2].ends_with("other.rs:1:1"),
        "second location in the same unopened file must resolve independently, got {:?}",
        rows[2]
    );
    assert!(
        rows[3].ends_with("missing.rs:1"),
        "a target file that can't be read must degrade to path:line, no column, got {:?}",
        rows[3]
    );
    assert!(
        rows[4].ends_with("main.rs:6"),
        "a line past the target's content must degrade to path:line, no column, got {:?}",
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
    // The row's path and its column must come from the same URI parse. When
    // Scheme rendered the path by stripping "file://" itself, it had no
    // percent-decoding: the row read "a%20name.rs" while the column beside
    // it had been read out of "a name.rs" — one row naming a file it did
    // not measure.
    assert!(
        rows[6].ends_with("a name.rs:1:3"),
        "a percent-encoded uri must render its decoded path and the column \
         read from that same file, got {:?}",
        rows[6]
    );
}
