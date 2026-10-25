// `publishDiagnostics` for a path the editor keys a buffer under in another
// form: the publish still finds the buffer that file's path opened.

use crate::editor::tests::lsp_rig::LspRig;

/// A publish for a file not on disk yet, named through a symlinked
/// directory, lands on the new-file buffer for that file.
#[test]
fn a_publish_under_a_symlinked_path_finds_a_new_file_buffer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let sid = rig.sid("rust-analyzer");
    let new_file = rig.root.join("src/new.rs");
    rig.ed
        .execute_typed("e", Some(new_file.to_str().unwrap()))
        .unwrap();
    rig.ed.settle();
    let bid = rig.ed.focused_buffer_id();
    std::os::unix::fs::symlink(rig.root.join("src"), rig.root.join("linkdir")).unwrap();

    let params = lsp_types::PublishDiagnosticsParams {
        uri: hume_lsp::uri::path_to_uri(&rig.root.join("linkdir/new.rs")).unwrap(),
        diagnostics: vec![lsp_types::Diagnostic::new_simple(
            lsp_types::Range::default(),
            "problem".to_string(),
        )],
        version: None,
    };

    assert_eq!(rig.ed.ingest_typed_publish_for_test(sid, params), Some(bid));
}
