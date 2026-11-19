//! A Markdown language server's link paths, written from its workspace
//! root, rewritten relative to the buffer's file.

use super::*;
use crate::editor::tests::lsp_rig::{LspRig, RigSpec};
use hume_lsp::test_util::RecordingLspBackend;

const MARKSMAN: &str = r#"(%define-language! "markdown" '("md") '() '()) (%configure-language! "markdown" (hash 'roots '(".marksman.toml")))
(register-lsp-server! "marksman" #:command "marksman")
(set-language-servers! "markdown" '("marksman"))
(load-plugin! "core:stdlib")
(load-plugin! "core:markdown")"#;

/// An item replacing `start..end` of the first line with `new_text`.
fn link_item(label: &str, new_text: &str, (start, end): (u32, u32)) -> serde_json::Value {
    serde_json::json!({
        "label": label,
        "textEdit": {
            "range": {"start": {"line": 0, "character": start},
                      "end": {"line": 0, "character": end}},
            "newText": new_text
        }
    })
}

/// `a/b/c.md` holding `marked`, under a root with `README.md` and
/// `a/b/d.md`, with `core:lsp` and `core:markdown` loaded and marksman
/// answering one completion request with `items`.
fn marksman_rig(tmp: &Path, marked: &str, items: serde_json::Value) -> (LspRig, RealRuntimeDirs) {
    let guard = RealRuntimeDirs::new();
    std::fs::create_dir_all(tmp.join("a/b")).unwrap();
    for file in ["README.md", "a/b/d.md"] {
        std::fs::write(tmp.join(file), "").unwrap();
    }
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"completionProvider": {}}}),
    );
    backend.respond_to(
        "textDocument/completion",
        serde_json::json!({"isIncomplete": false, "items": items}),
    );
    let init = format!(
        "{MARKSMAN}\n{}",
        hume_scripting::testing::eager_load_scm("core:lsp", None)
    );
    let spec = RigSpec {
        language: "markdown",
        extension: "md",
        file: "a/b/c.md",
        marked,
        markers: &[".marksman.toml"],
        init: &init,
        dirs: guard.dirs(),
    };
    let mut rig = LspRig::drained(tmp, spec, backend);
    rig.ed.settle();
    (rig, guard)
}

/// The two rooted paths marksman offers for `[x](|)`: a sibling and the
/// root's README.
fn sibling_and_readme() -> serde_json::Value {
    serde_json::json!([
        link_item("a/b/d.md", "/a/b/d.md", (4, 4)),
        link_item("README.md", "/README.md", (4, 4)),
    ])
}

fn complete(rig: &mut LspRig) {
    rig.ed.feed_key(key('i'));
    rig.ed.settle();
    trigger_completion(&mut rig.ed);
    rig.ed.drain_lsp();
    rig.ed.settle();
}

#[test]
fn a_rooted_link_path_is_offered_relative_to_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = marksman_rig(tmp.path(), "[x](-[)]>\n", sibling_and_readme());
    complete(&mut rig);
    let mut labels = menu_labels(&rig.ed);
    labels.sort();
    assert_eq!(
        labels,
        ["../../README.md", "c.md", "d.md"],
        "the plugin's own d.md is the server's, shown once"
    );
}

#[test]
fn accepting_a_rewritten_path_writes_it_relative() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = marksman_rig(tmp.path(), "[x](-[)]>\n", sibling_and_readme());
    complete(&mut rig);
    for ch in ['R', 'E'] {
        rig.ed.feed_key(key(ch));
    }
    rig.ed.settle();
    rig.ed.feed_key(key_enter());
    rig.ed.settle();
    assert_eq!(rig.ed.doc().text().to_string(), "[x](../../README.md)\n");
}

/// What `lsp-completion/markdown` makes of `item`, as its `newText`,
/// `filterText` and label.
fn rewritten(item: serde_json::Value) -> Vec<String> {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = marksman_rig(tmp.path(), "[x](-[)]>\n", serde_json::json!([]));
    let answer = serde_json::json!({"isIncomplete": false, "items": [item]}).to_string();
    rig.probe(&format!(
        r#"(let ([item (json-ref (call! "lsp-completion/markdown" pane (car (lsp-servers pane))
                                       (json-parse {answer:?}))
                                "items" 0)])
             (for-each (lambda (key) (log! 'warning (string-append "probe: " (apply json-ref item key))))
                       '(("textEdit" "newText") ("filterText") ("label"))))"#
    ));
    rig.warnings()
        .into_iter()
        .filter_map(|w| w.strip_prefix("probe: ").map(str::to_string))
        .collect()
}

#[test]
fn a_heading_anchor_stays_after_the_rewritten_path() {
    let mut item = link_item("a/b/d.md / Usage", "/a/b/d.md#usage", (4, 4));
    item["filterText"] = "/a/b/d.md#Usage".into();
    assert_eq!(
        rewritten(item),
        ["d.md#usage", "d.md#Usage", "d.md / Usage"]
    );
}

#[test]
fn a_whole_link_the_server_writes_has_its_path_rewritten() {
    assert_eq!(
        rewritten(link_item("a/b/d.md", "[x](/a/b/d.md)", (0, 4))),
        ["[x](d.md)", "[x](d.md)", "d.md"]
    );
}
