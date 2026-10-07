// References: `lsp-references`, reusing the goto-definition family's
// worker shape but always presenting the drawer (never auto-jumping even
// for a single result). Loads the real shipped `core:lsp` plugin in place
// (`RealRuntimeDirs`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;

/// A [`core_lsp_rig`] over "fn main() {\n    foo();\n}\n" with a
/// references provider.
fn setup(
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs, ServerId) {
    let (rig, guard) = core_lsp_rig(
        tmp,
        "-[f]>n main() {\n    foo();\n}\n",
        serde_json::json!({"capabilities": {"referencesProvider": true}}),
        configure,
    );
    let sid = rig.sid("rust-analyzer");
    (rig.ed, guard, sid)
}

fn run_references(ed: &mut Editor) {
    // lsp-references is key-bindable, not typed, so dispatch through the
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

#[test]
fn three_locations_list_three_rows() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    run_references(&mut ed);

    assert_eq!(drawer_rows(&ed).len(), 3);
}

#[test]
fn enter_jumps_and_drawer_stays_open() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    run_references(&mut ed);
    render(&mut ed); // establishes real geometry so Ctrl-d below isn't a pre-frame no-op
    ed.handle_key(key_ctrl('d'));
    ed.handle_key(key_enter());
    ed.settle();

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(1),
        "Enter on row 2 must jump to that entry's line"
    );
    assert!(
        ed.state.input.drawer().is_some(),
        "the references drawer must stay open after a jump (drawer browse behavior)"
    );
}

#[test]
fn single_result_still_opens_the_drawer() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 1, 4)]),
        );
    });

    run_references(&mut ed);

    assert!(
        ed.state.input.drawer().is_some(),
        "references must always drawer-list, even a single result (unlike goto's auto-jump)"
    );
}

#[test]
fn null_result_reports_no_references() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/references", serde_json::Value::Null);
    });

    run_references(&mut ed);

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no references"),
        "expected a no-references message, got {msg:?}"
    );
}

/// The user is free to switch buffers while a `textDocument/references`
/// request is in flight, the same async-round-trip race `lsp-hover`'s own
/// `#:require-focus` guards against. Unlike single-location goto (a
/// navigation the user asked for, completed regardless of focus),
/// references always opens a drawer: cursor-anchored UI that must not
/// appear over whatever the user switched to.
#[test]
fn stale_response_after_a_buffer_switch_opens_no_drawer() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    // Sends the request synchronously; no settle() before the
    // switch below, the same technique as `lsp_hover.rs`'s own
    // `stale_response_after_a_buffer_switch_shows_no_popup`.
    ed.execute_keymap_command("lsp-references".into(), Some(1), false);

    let other = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other, "\n").unwrap();
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();

    ed.drain_lsp();
    ed.settle();

    assert!(
        ed.state.views.drawer.read().is_none(),
        "a references response for a buffer that's no longer focused must not open a drawer"
    );
}

#[test]
fn buffer_with_no_path_reports_and_tracks_nothing() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 0, 0)]),
        );
    });
    ed.doc_mut().set_path(None);

    run_references(&mut ed);

    assert_eq!(ed.state.panes.tracked.len(), 0);
    assert!(ed.state.views.drawer.read().is_none());
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no file"),
        "expected a no-file message, got {msg:?}"
    );
}
