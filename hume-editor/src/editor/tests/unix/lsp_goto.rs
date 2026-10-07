// Goto definition family: `lsp-goto-definition` /
// `-declaration` / `-type-definition` / `-implementation`, composing
// `lsp-request!`, `lsp-capabilities`, `goto-location!`,
// `show-drawer-list!` (via locations.scm's lsp/show-locations!). Loads the real
// shipped `core:lsp` plugin in place (`RealRuntimeDirs`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;

/// Three lines, so a `Location` can point at a different line for the
/// jump-back test.
const FIXTURE: &str = "fn main() {\n    foo();\n}\n";

/// A [`core_lsp_rig`] over `content` (cursor at its start), its server
/// providing every goto method, with a driven handshake so
/// `lsp-capabilities` decodes.
fn setup(
    tmp: &Path,
    content: &str,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs, ServerId) {
    let (rig, guard) = core_lsp_rig(
        tmp,
        &marked_at_start(content),
        serde_json::json!({"capabilities": {
            "definitionProvider": true, "declarationProvider": true,
            "typeDefinitionProvider": true, "implementationProvider": true
        }}),
        configure,
    );
    let sid = rig.sid("rust-analyzer");
    (rig.ed, guard, sid)
}

/// `cmd` carries a leading `:` for caller readability (`":lsp-goto-definition"`)
/// even though every `lsp-goto-*` command is key-bindable, not typed:
/// dispatched here through the keymap pipeline, the way its bound key would.
fn run_goto(ed: &mut Editor, cmd: &str) {
    let name = cmd.strip_prefix(':').unwrap_or(cmd);
    ed.execute_keymap_command(name.to_owned().into(), Some(1), false);
    // Settle now (mirrors the real interactive loop, which drains after
    // every keystroke) before the async response arrives: same ordering
    // fix as lsp_hover.rs.
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

fn location_link(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "targetUri": uri,
        "targetRange": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character + 3}},
        "targetSelectionRange": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character + 3}}
    })
}

#[test]
fn null_result_reports_no_definition_found() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to("textDocument/definition", serde_json::Value::Null);
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no definition found"),
        "expected a no-definition message, got {msg:?}"
    );
}

#[test]
fn single_location_hashmap_jumps_directly() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&uri, 1, 4));
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(1),
        "a single Location must jump directly to line 1 (0-indexed)"
    );
}

#[test]
fn wire_target_inside_a_combining_sequence_snaps_to_the_clusters_start() {
    // "e\u{0301}" (e + combining acute) is one grapheme cluster, two chars,
    // both single UTF-16 code units, so character=1 is a perfectly valid
    // wire position (no surrogate-pair splitting involved) that still lands
    // *inside* the cluster: between its base character and its combining
    // mark. The day-one grapheme invariant says a cursor may never sit
    // there.
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), "e\u{0301}\n", |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&uri, 0, 1));
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.current_view().primary().head().offset(),
        co(0),
        "a wire position between a base char and its combining mark must snap to the cluster's start"
    );
}

#[test]
fn single_element_array_jumps_directly() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 1, 4)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(1),
        "a length-1 Location[] must jump directly, not open the drawer"
    );
    assert!(ed.state.input.drawer().is_none());
}

#[test]
fn multi_element_array_opens_the_drawer_and_row_select_jumps() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let rows = drawer_rows(&ed);
    assert_eq!(rows.len(), 3);

    // Select row index 1 (the second entry, line 1).
    render(&mut ed); // establishes real geometry so Ctrl-d below isn't a pre-frame no-op
    ed.handle_key(key_ctrl('d'));
    ed.handle_key(key_enter());
    ed.settle();

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(1),
        "selecting row 2 in the drawer must jump to that entry's line"
    );
}

/// A Windows drive-letter `file://` URI (`file:///C:/...`) must display in
/// the drawer without a leading `/` before the drive letter.
/// `hume_lsp::uri::uri_to_path`'s plain slash strip alone leaves one in
/// ("/C:/foo"), not a valid Windows path; `uri_to_display_string` is the
/// sibling that additionally strips it before a drive letter, which is what
/// `location_display_parts` renders the drawer row through. The second
/// location's file need not exist: opening the drawer only formats row
/// labels, it doesn't touch the filesystem (only selecting a row and jumping
/// would).
#[test]
fn windows_drive_letter_uri_displays_without_leading_slash() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let win_uri = "file:///C:/Users/x/main.rs";
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), loc(win_uri, 1, 0)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let rows = drawer_rows(&ed);
    assert_eq!(rows.len(), 2);
    assert!(
        rows[1].starts_with("C:/Users/x/main.rs"),
        "Windows drive-letter URI must display without a leading '/' \
         before the drive letter, got {:?}",
        rows[1]
    );
}

/// The multi-entry-array case exercises `lsp-locations->display-parts`'s
/// `LocationLink` decoding (`hume_lsp::location::decode_location`'s
/// `targetUri`/`targetSelectionRange` branch), unlike the single-entry jump
/// tests above, which reach the same decoder through `goto-location!`
/// instead.
#[test]
fn multi_element_location_link_array_opens_the_drawer_and_row_select_jumps() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([
                location_link(&uri, 0, 0),
                location_link(&uri, 1, 4),
                location_link(&uri, 2, 0)
            ]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let rows = drawer_rows(&ed);
    assert_eq!(rows.len(), 3);
    assert!(
        rows[1].ends_with(":2:5"),
        "row text must be built from targetSelectionRange (1-based line:col), got {:?}",
        rows[1]
    );

    // Select row index 1 (the second entry, line 1).
    render(&mut ed); // establishes real geometry so Ctrl-d below isn't a pre-frame no-op
    ed.handle_key(key_ctrl('d'));
    ed.handle_key(key_enter());
    ed.settle();

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(1),
        "selecting a LocationLink drawer row must jump to targetSelectionRange's line"
    );
}

#[test]
fn location_link_array_prefers_target_selection_range() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([location_link(&uri, 1, 4)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(1),
        "a single-entry LocationLink[] must jump using targetSelectionRange"
    );
}

#[test]
fn jump_back_returns_to_the_origin_after_a_jump() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&uri, 1, 4));
    });
    let before = state(&ed);

    run_goto(&mut ed, ":lsp-goto-definition");
    assert_ne!(
        state(&ed),
        before,
        "sanity: the jump must have moved the cursor"
    );

    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        state(&ed),
        before,
        "Ctrl-o must return to the pre-jump position"
    );
}

/// `goto-location!`'s wire (`Location` hashmap) path opens the target file
/// via `lsp::edits::resolve_or_open` → `buffer::lifecycle::
/// open_or_dedup_and_notify` when it isn't already open, which can't detect
/// language inline (see that function's doc), so it queues the buffer onto
/// `EditorState.pending_language_detection`, drained at the tail of
/// `apply_script_effects` once this eval (`run_goto`'s `:lsp-goto-definition`
/// dispatch) returns.
///
/// If `resolve_or_open` called the bare `lifecycle::open_or_dedup`, the
/// newly-opened buffer would never get a `language`.
#[test]
fn goto_to_an_unopened_file_detects_its_language() {
    let tmp = safe_tempdir();
    let other_file = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other_file, "fn other() {}\n").unwrap();
    let other_canonical = std::fs::canonicalize(&other_file).unwrap();
    let other_uri = hume_lsp::uri::path_to_uri(&other_canonical)
        .unwrap()
        .as_str()
        .to_string();

    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, move |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&other_uri, 0, 3));
    });
    run_goto(&mut ed, ":lsp-goto-definition");

    let bid = ed
        .state
        .buffers
        .find_by_path(&other_canonical)
        .expect("goto-location! must have opened the target file");
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust"),
        "the goto-opened file must have its language detected"
    );
}

/// A wire `Location`'s server encoding must come from the buffer that sent
/// the request (`bid`, captured by `lsp/goto-request` before the request
/// went out), never from whatever is focused when the response callback
/// happens to run. An LSP round-trip is async, so the user is free to
/// switch buffers while it's in flight.
///
/// `main.rs`'s line 1 is `let π = 1;`. Under this test's UTF-8-negotiated
/// server, wire `character: 7` (a byte offset) decodes to char offset 6
/// (`=`, since `π` occupies 2 bytes but 1 char); under the UTF-16 default
/// (code-unit offset), the same wire value decodes to char offset 7 (the
/// space after `=`). The test switches focus to an unrelated, server-less
/// buffer between sending the request and draining its response. If
/// `resolve_goto_target`'s Wire arm read that buffer's encoding (UTF-16
/// default) instead of `origin`'s, the cursor would land at char offset 7
/// instead of 6.
#[test]
fn wire_response_decodes_with_the_requesting_buffers_encoding_not_live_focus() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (rig, guard) = core_lsp_rig(
        tmp.path(),
        "-[f]>n main() {\nlet \u{3c0} = 1;\n}\n",
        serde_json::json!({"capabilities": {
            "definitionProvider": true,
            "positionEncoding": "utf-8"
        }}),
        |backend, _sid| backend.respond_to("textDocument/definition", loc(&uri, 1, 7)),
    );
    let bid_a = rig.bid;
    let mut ed = rig.ed;

    // Send the request from `main.rs` (`bid_a`, UTF-8 server). Dispatches
    // synchronously, so the request has already left with `bid_a` captured
    // by the time this returns. No `settle()` here:
    // `InlineLspBackend::send` queues the canned response for the *next*
    // drain rather than answering inline, but `settle()` itself drains LSP
    // (`drain_async_sources` → `drain_lsp`): calling it now would close the
    // race window before this test ever opens it.
    ed.execute_keymap_command("lsp-goto-definition".into(), Some(1), false);

    // Switch focus to an unrelated, server-less buffer *before* the
    // response arrives. This is the race window. The rig registers
    // a server for `rust` buffers only, so none attaches to a `.txt` one.
    let other = rig_root(tmp.path()).join("other.txt");
    std::fs::write(&other, "\n").unwrap();
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();
    assert_ne!(
        ed.focused_buffer_id(),
        bid_a,
        "sanity: focus must have actually moved before the response drains"
    );

    // Now let the response land.
    ed.drain_lsp();
    ed.settle();

    let line1_start = hume_rope::lines::line_start_char(
        ed.doc().text().rope(),
        hume_rope::line::RopeyLine::new(1),
    );
    let head = ed.current_view().primary().head().offset();
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "goto must have jumped back into main.rs"
    );
    assert_eq!(
        head.chars_since(line1_start),
        6,
        "must decode with main.rs's own UTF-8 server, landing on '=' (char 6), \
         not the UTF-16 default the newly-focused buffer would fall back to (char 7)"
    );

    drop(guard);
}

/// Several targets arriving after the requesting pane moved to another buffer
/// still open the drawer.
#[test]
fn multi_element_response_after_a_buffer_switch_opens_the_drawer() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    ed.execute_keymap_command("lsp-goto-definition".into(), Some(1), false);
    let other = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other, "\n").unwrap();
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();
    ed.drain_lsp();
    ed.settle();

    assert_eq!(drawer_rows(&ed).len(), 3);
}

/// A target whose path can't be opened (here: it's a directory,
/// not a file; `Buffer::from_file_or_new` only tolerates `NotFound`) must
/// still error and leave the cursor untouched.
#[test]
fn goto_target_is_directory_errors_without_moving_the_cursor() {
    let tmp = safe_tempdir();
    let dir_uri = hume_lsp::uri::path_to_uri(&std::fs::canonicalize(tmp.path()).unwrap())
        .unwrap()
        .as_str()
        .to_string();
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&dir_uri, 0, 0));
    });
    let before = state(&ed);

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(state(&ed), before, "a failed goto must not move the cursor");
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("error") || msg.to_lowercase().contains("goto-location"),
        "expected an error message, got {msg:?}"
    );
}

/// A target whose file doesn't exist yet (but whose path is otherwise valid)
/// must open a new-file buffer and jump to it: the same `:e newfile.txt`
/// tolerance `resolve_or_open` shares with `Editor::resolve_open_path` via
/// `Buffer::from_file_or_new`, not an error. Covers a server-driven
/// definition/rename that points at a file it expects the client to create.
#[test]
fn goto_missing_target_opens_new_file_buffer_and_jumps_to_it() {
    let tmp = safe_tempdir();
    let missing = rig_root(tmp.path()).join("not_yet_created.rs");
    let missing_uri = format!("file://{}", missing.display());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&missing_uri, 0, 0));
    });
    let start_bid = ed.focused_buffer_id();

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_ne!(
        ed.focused_buffer_id(),
        start_bid,
        "goto must have switched to the new-file buffer"
    );
    assert!(
        ed.doc().is_new_file(),
        "target buffer must be a pending new-file buffer, not an error"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        !msg.to_lowercase().contains("error"),
        "must not report an error, got {msg:?}"
    );
}

/// A `Location` missing `range` errors rather than jumping to line 0: the
/// goto answer is decoded into rows (`lsp-locations->display-parts`) through
/// the same `hume_lsp::location::decode_location` `goto-location!` uses, so
/// the error comes from that decode.
#[test]
fn location_missing_range_errors_instead_of_jumping() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to("textDocument/definition", serde_json::json!({"uri": uri}));
    });
    let before = state(&ed);

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        state(&ed),
        before,
        "a malformed location must not move the cursor"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("lsp-locations->display-parts") && msg.contains("missing range"),
        "expected an error naming the builtin and the missing field, got {msg:?}"
    );
}

#[test]
fn each_command_sends_its_own_method() {
    // Wiring smoke test: script a distinctive response only for the exact
    // method each command should send. If a command sent the wrong method,
    // no response would be queued for it and the jump would never happen.
    for (cmd, method) in [
        ("lsp-goto-definition", "textDocument/definition"),
        ("lsp-goto-declaration", "textDocument/declaration"),
        ("lsp-goto-type-definition", "textDocument/typeDefinition"),
        ("lsp-goto-implementation", "textDocument/implementation"),
    ] {
        let tmp = safe_tempdir();
        let uri = rust_rig_uri(tmp.path());
        let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
            backend.respond_to(method, loc(&uri, 1, 4));
        });

        run_goto(&mut ed, &format!(":{cmd}"));

        assert_eq!(
            ed.doc()
                .text()
                .char_to_line(ed.current_view().primary().head().offset()),
            hume_rope::line::ContentLine::new(1),
            "{cmd} must send {method} and jump on its response"
        );
    }
}

/// A goto-definition landing on a different, unopened file is a
/// non-interactive `switch-to-buffer!` (via `goto-location!`): the switch
/// happens inside `drain_lsp`'s response handling, never through
/// `type_cmd`/`feed_key`, so it exercises `settle()`'s diff rather than any
/// command-side raise. Must raise exactly one `OnBufferEnter`, same as every
/// other focus-changing action.
///
/// If `goto-location!`'s buffer switch bypassed `settle()`'s diff, the trace
/// log would stay empty. A duplicate raise on the same switch would add a
/// second entry.
#[test]
fn goto_into_another_file_raises_exactly_one_on_buffer_enter() {
    let tmp = safe_tempdir();
    let other_file = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other_file, "fn other() {}\n").unwrap();
    let other_canonical = std::fs::canonicalize(&other_file).unwrap();
    let other_uri = hume_lsp::uri::path_to_uri(&other_canonical)
        .unwrap()
        .as_str()
        .to_string();

    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, move |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&other_uri, 0, 3));
    });

    // Drain the startup OnBufferEnter (no handler registered yet, so
    // nothing gets logged for it) before installing the counting handler.
    ed.settle();
    let mut host = ed.scripting.take().expect("setup() installs a host");
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-enter (lambda (bid) (log! 'trace "entered")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    run_goto(&mut ed, ":lsp-goto-definition");

    let bid = ed
        .state
        .buffers
        .find_by_path(&other_canonical)
        .expect("goto-location! must have opened and switched to the target file");
    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "sanity: the goto must have landed on the other file"
    );
    let entered = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "entered")
        .count();
    assert_eq!(
        entered, 1,
        "a goto-definition switch into another file must raise exactly one OnBufferEnter"
    );
}

// ── The request's tracked position ──────────────────────────────────────────

/// A reply whose locations cannot be listed raises in the callback, and the
/// position the request tracked is released anyway.
#[test]
fn a_malformed_multi_location_reply_releases_the_tracked_position() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), {"uri": uri}]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(ed.state.panes.tracked.len(), 0);
}

/// A reply dropped because the text changed after the request never reaches
/// the callback, and the position the request tracked is released.
#[test]
fn a_reply_dropped_as_stale_releases_the_tracked_position() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4)]),
        );
    });

    ed.execute_keymap_command("lsp-goto-definition".into(), Some(1), false);
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    ed.drain_lsp();
    ed.settle();

    assert_eq!(ed.state.panes.tracked.len(), 0);
}

/// The drawer keeps the position for its refresh.
#[test]
fn an_opened_drawer_keeps_the_tracked_position() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(drawer_rows(&ed).len(), 2);
    assert_eq!(ed.state.panes.tracked.len(), 1);
}

/// A second list replaces the first drawer, whose `#f` is delivered after
/// the new session is stored: the new session keeps its tracked position.
#[test]
fn a_second_list_keeps_its_session_when_the_replaced_drawer_closes() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        for _ in 0..2 {
            backend.respond_to(
                "textDocument/definition",
                serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4)]),
            );
        }
    });

    run_goto(&mut ed, ":lsp-goto-definition");
    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(drawer_rows(&ed).len(), 2);
    assert_eq!(ed.state.panes.tracked.len(), 1);
}

/// A buffer with no path has no document to ask about: the command reports
/// it, sends nothing and tracks no position.
#[test]
fn buffer_with_no_path_reports_and_tracks_nothing() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), FIXTURE, |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0)]),
        );
    });
    ed.doc_mut().set_path(None);

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(ed.state.panes.tracked.len(), 0);
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no file"),
        "expected a no-file message, got {msg:?}"
    );
}
