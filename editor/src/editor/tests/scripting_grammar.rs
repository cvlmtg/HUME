// Editor-level tests for the tree-sitter grammar wiring: setup_buffer_syntax,
// reparse_stale_buffers, sweep_buffers_for_grammars, and the register-grammar!
// Steel builtin (command-mode branch).
//
// All tests require grammar fixtures built by `scripts/fetch-test-grammars.sh`.
// On a fixture-less checkout the helpers panic with a clear install message.
// CI installs fixtures before running tests so panic never fires there.

use super::*;

use std::path::PathBuf;

use crate::editor::keymap::Keymap;
use crate::scripting::ScriptingHost;
use crate::settings::EditorSettings;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn grammar_fixture(name: &str) -> (PathBuf, PathBuf) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars");
    let suffix = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(windows) {
        "dll"
    } else {
        "so"
    };
    let parser = base.join(name).join(format!("parser.{suffix}"));
    if !parser.exists() {
        panic!(
            "grammar fixture missing: {}\ninstall the tree-sitter CLI (npm i -g tree-sitter-cli) and run scripts/fetch-test-grammars.sh from the repo root",
            parser.display()
        );
    }
    (parser, base.join(name).join("queries/highlights.scm"))
}

// ---------------------------------------------------------------------------
// Direct-attach tests (Rust API only; no Steel dispatch)
// ---------------------------------------------------------------------------

/// Flip: without attach_grammar the grammar field is None so setup_buffer_syntax
/// returns early — all three handles stay None.
#[test]
fn attach_then_set_language_attaches_syntax() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    ed.set_buffer_language(bid, Some("json".to_owned()));
    assert!(ed.buffers.get(bid).parser.is_some(), "parser must be set after attach");
    assert!(ed.engine_view.buffers[bid].syntax.is_some(), "engine syntax must be set");
    assert!(ed.engine_view.buffers[bid].tree.is_some(), "engine tree must be set");
}

/// Flip: if clear didn't propagate, parser/syntax/tree would still be Some after set(None).
#[test]
fn clear_language_detaches_syntax_keeps_identity() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    ed.set_buffer_language(bid, Some("json".to_owned()));
    assert!(ed.buffers.get(bid).parser.is_some());

    ed.set_buffer_language(bid, None);
    assert!(ed.buffers.get(bid).parser.is_none(), "parser must be cleared on language=None");
    assert!(ed.engine_view.buffers[bid].syntax.is_none(), "syntax must be cleared");
    assert!(ed.engine_view.buffers[bid].tree.is_none(), "tree must be cleared");
    // Identity survives detach — grammar is gone, language definition is not.
    assert!(ed.languages.by_name("json").is_some(), "identity must survive grammar detach");
}

/// Flip: if sweep ignored the name filter it would attach after the rust-sweep midpoint.
#[test]
fn sweep_attaches_syntax_on_matching_language() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    // Set language BEFORE grammar is attached — no syntax yet.
    ed.set_buffer_language(bid, Some("json".to_owned()));
    assert!(ed.buffers.get(bid).parser.is_none(), "no grammar → parser must be absent");

    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    ed.sweep_buffers_for_grammars(vec!["json".to_owned()]);
    assert!(ed.buffers.get(bid).parser.is_some(), "sweep must attach parser when language matches");
}

/// Flip: if sweep applies to all buffers regardless of name, the first assert would fail.
#[test]
fn sweep_no_op_for_nonmatching_language() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    // Set language but don't attach grammar yet — parser stays absent.
    ed.set_buffer_language(bid, Some("json".to_owned()));
    assert!(ed.buffers.get(bid).parser.is_none());

    // Sweep for a different language — must leave the json buffer untouched.
    ed.sweep_buffers_for_grammars(vec!["rust".to_owned()]);
    assert!(
        ed.buffers.get(bid).parser.is_none(),
        "wrong-language sweep must not attach parser for json buffer",
    );

    // Sanity flip: sweeping "json" does attach.
    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    ed.sweep_buffers_for_grammars(vec!["json".to_owned()]);
    assert!(ed.buffers.get(bid).parser.is_some(), "correct-language sweep must attach parser");
}

/// Flip: without reparse_stale_buffers the parsed_gen would stay at gen0 even after the edit.
#[test]
fn reparse_advances_parsed_gen_after_edit() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    ed.set_buffer_language(bid, Some("json".to_owned()));

    // setup_buffer_syntax sets parsed_gen = text_gen.
    let gen0 = ed.buffers.get(bid).text_gen;
    assert_eq!(
        ed.buffers.get(bid).parser.as_ref().unwrap().parsed_gen,
        gen0,
        "parsed_gen must equal text_gen after initial setup",
    );

    // Insert a character — bumps text_gen.
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_esc());
    let gen1 = ed.buffers.get(bid).text_gen;
    assert!(gen1 > gen0, "edit must bump text_gen");
    assert_eq!(
        ed.buffers.get(bid).parser.as_ref().unwrap().parsed_gen,
        gen0,
        "parsed_gen must lag behind text_gen before reparse",
    );

    ed.reparse_stale_buffers();
    assert_eq!(
        ed.buffers.get(bid).parser.as_ref().unwrap().parsed_gen,
        gen1,
        "reparse must advance parsed_gen to current text_gen",
    );

    // Second call is a no-op — parsed_gen stays at gen1.
    ed.reparse_stale_buffers();
    assert_eq!(
        ed.buffers.get(bid).parser.as_ref().unwrap().parsed_gen,
        gen1,
        "second reparse must be a no-op when gen already matches",
    );
}

/// Flip: without the max_bytes gate, parser would still be Some after reparse.
#[test]
fn reparse_detaches_when_buffer_exceeds_max_bytes() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    ed.set_buffer_language(bid, Some("json".to_owned()));
    assert!(ed.buffers.get(bid).parser.is_some(), "parser must be set initially");

    // Any non-empty buffer exceeds 1 byte.
    ed.settings.syntax_highlight_max_bytes = 1;
    ed.reparse_stale_buffers();
    assert!(ed.buffers.get(bid).parser.is_none(), "parser must detach when exceeding max_bytes");
    assert!(ed.engine_view.buffers[bid].syntax.is_none(), "syntax must be cleared");
}

// ---------------------------------------------------------------------------
// Steel command-mode test
// ---------------------------------------------------------------------------

/// End-to-end: register-grammar! in command mode attaches the grammar immediately
/// and the execute path auto-sweeps open buffers of that language.
///
/// Exercises: register_grammar command branch → attach_grammar → theme.bake →
/// pending_grammar_sweeps → SteelCmdResult.grammar_sweeps → sweep_buffers_for_grammars
/// → setup_buffer_syntax → engine SharedBuffer.syntax.
///
/// Flip: if the command body ran in init mode (queuing instead of attaching),
/// no sweep would fire and syntax would stay None.
#[test]
#[cfg(not(windows))]
fn register_grammar_command_mode_attaches_and_sweeps() {
    let (parser, hl) = grammar_fixture("json");
    let tmp = tempfile::tempdir().unwrap();
    let init_path = tmp.path().join("init.scm");
    // Embed absolute paths directly — safe on macOS/Linux (no backslashes).
    std::fs::write(
        &init_path,
        format!(
            r#"(define-command! "attach-json" "Attach JSON grammar" (lambda () (register-grammar! "json" "{}" "tree_sitter_json" "{}")))"#,
            parser.display(),
            hl.display(),
        ),
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let cmds = host.eval_init(&init_path, &mut s, &mut km, Default::default()).expect("eval_init");

    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.register_steel_cmds(cmds);
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    ed.set_buffer_language(bid, Some("json".to_owned()));
    assert!(ed.buffers.get(bid).parser.is_none(), "no grammar attached yet");

    ed.scripting = Some(host);
    type_cmd(&mut ed, ":attach-json");

    assert!(ed.languages.has_grammar("json"), "has_grammar must be true after attach");
    assert!(
        ed.engine_view.buffers[bid].syntax.is_some(),
        "buffer syntax must be set after command-mode register-grammar! + auto-sweep",
    );
}

// ---------------------------------------------------------------------------
// has_grammar reflection
// ---------------------------------------------------------------------------

/// Flip: if has_grammar ignored grammar presence it would return true for identity-only.
#[test]
fn language_has_grammar_false_for_identity_only_true_after_attach() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[a]>b\n");
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    assert!(!ed.languages.has_grammar("json"), "identity without grammar → has_grammar false");
    assert!(!ed.languages.has_grammar("unknown"), "unknown language → has_grammar false");

    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    assert!(ed.languages.has_grammar("json"), "has_grammar must be true after attach");
}

// ---------------------------------------------------------------------------
// Fix 1 — replace_buffer_in_place must clear stale engine syntax state
// ---------------------------------------------------------------------------

/// Regression: replace_buffer_in_place used to leave ev.buffers[id].tree / .syntax
/// pointing at the old content. Without the fix, both would still be Some after
/// replacing with a scratch buffer.
///
/// Flip: if the engine-side clear is removed, the two `.is_none()` asserts fail.
#[test]
fn replace_buffer_in_place_clears_engine_syntax_state() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    ed.set_buffer_language(bid, Some("json".to_owned()));
    assert!(ed.engine_view.buffers[bid].tree.is_some(), "tree must be set before replace");
    assert!(ed.engine_view.buffers[bid].syntax.is_some(), "syntax must be set before replace");

    // Replace with a scratch buffer (no path, language=None). detect_and_set_language
    // returns None → set_buffer_language no-ops; the engine-side clear in
    // ops::replace_buffer_in_place is the load-bearing cleanup here.
    ed.replace_buffer_in_place(bid, Buffer::scratch());

    assert!(ed.engine_view.buffers[bid].tree.is_none(), "stale tree must be cleared on replace");
    assert!(ed.engine_view.buffers[bid].syntax.is_none(), "stale syntax must be cleared on replace");
    assert!(ed.buffers.get(bid).parser.is_none(), "parser must be cleared on replace");
}

// ---------------------------------------------------------------------------
// Fix 3 — reparse_stale_buffers must re-attach on shrink below cap
// ---------------------------------------------------------------------------

/// Regression: once a buffer's parser was detached (via the max_bytes growth branch),
/// reparse_stale_buffers used to skip it forever (`None => continue`). Without the
/// fix, the second `reparse_stale_buffers` call leaves parser=None.
///
/// Flip: if the re-attach branch is removed, the final `parser.is_some()` assert fails.
#[test]
fn reparse_reattaches_after_shrink_under_cap() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.languages.register_identity("json", &["json"], &[], &[]).unwrap();
    ed.languages
        .attach_grammar("json", &parser, "tree_sitter_json", &hl, &mut ed.engine_view.registry)
        .unwrap();
    ed.set_buffer_language(bid, Some("json".to_owned()));
    assert!(ed.buffers.get(bid).parser.is_some(), "parser must be set initially");

    // Force detach by setting a 1-byte cap — any non-empty buffer exceeds it.
    ed.settings.syntax_highlight_max_bytes = 1;
    ed.reparse_stale_buffers();
    assert!(ed.buffers.get(bid).parser.is_none(), "parser must detach when exceeding cap");
    assert!(ed.engine_view.buffers[bid].syntax.is_none(), "syntax must be cleared on detach");

    // Restore a generous cap — next reparse must re-attach.
    ed.settings.syntax_highlight_max_bytes = usize::MAX;
    ed.reparse_stale_buffers();
    assert!(
        ed.buffers.get(bid).parser.is_some(),
        "parser must re-attach when buffer shrinks back under cap",
    );
    assert!(
        ed.engine_view.buffers[bid].syntax.is_some(),
        "engine syntax must be rebuilt after re-attach",
    );
}
