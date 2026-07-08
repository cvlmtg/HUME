// Editor-level tests for tree-sitter injection support (M11): real markdown +
// markdown.inline + rust grammar fixtures, exercised through the full
// setup_buffer_syntax / reparse_stale_buffers / bake_pending_edits path.
//
// Requires scripts/fetch-test-grammars.sh (markdown, markdown.inline, rust).

use super::*;

use crate::editor::tests::{grammar_parser_path, grammar_query_path, helix_injections_path};

/// Attach the fixture grammar `name` (source name == attach identity — true
/// for every real PLUM install; there is no renaming split in production).
/// `injections` selects between the real Helix-maintained injections.scm
/// (what PLUM actually installs) and none.
fn attach(ed: &mut Editor, name: &str, symbol: &str, injections: bool) {
    let parser_path = grammar_parser_path(name);
    let hl_path = grammar_query_path(name);
    let inj_path = injections.then(|| helix_injections_path(name)).flatten();
    ed.state
        .languages
        .attach_grammar(
            name,
            &parser_path,
            symbol,
            &hl_path,
            inj_path.as_deref(),
            &mut ed.view.registry,
        )
        .unwrap_or_else(|e| panic!("attach {name}: {e}"));
}

fn fixtures_present() -> bool {
    ["markdown", "markdown.inline", "rust"]
        .iter()
        .all(|n| grammar_parser_path(n).exists())
        && helix_injections_path("markdown").is_some()
}

/// Build an editor with markdown (+ the real Helix-maintained
/// `injections.scm` — what `:plum-install-grammar` actually fetches, not
/// the grammar's own bundled query), the `markdown.inline` grammar (the
/// name Helix's injections.scm resolves `(inline)` to — same name as the
/// fixture directory, no renaming needed), and rust, then attach markdown as
/// the buffer's language and drain the initial parse.
///
/// Builds the buffer directly (cursor at 0 via `SelectionSet::default()`)
/// rather than through the `editor_from` marker DSL — this file's tests
/// mostly care about byte-offset edits at position 0, not cursor placement.
fn markdown_editor(source: &str) -> (Editor, hume_engine::pipeline::BufferId) {
    let buf = crate::editor::buffer::Buffer::new(
        hume_editing::text::Text::from(source),
        hume_editing::selection::SelectionSet::default(),
    );
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();

    ed.state
        .languages
        .register_identity("markdown", &["md"], &[], &[])
        .unwrap();
    attach(&mut ed, "markdown", "tree_sitter_markdown", true);
    ed.state
        .languages
        .register_identity("markdown.inline", &[], &[], &[])
        .unwrap();
    attach(
        &mut ed,
        "markdown.inline",
        "tree_sitter_markdown_inline",
        false,
    );
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    attach(&mut ed, "rust", "tree_sitter_rust", false);

    ed.set_buffer_language(bid, Some("markdown".to_owned()));
    ed.reparse_stale_buffers(); // drains the initial full parse
    (ed, bid)
}

#[test]
fn markdown_buffer_installs_root_plus_injected_layers() {
    if !fixtures_present() {
        return; // scripts/fetch-test-grammars.sh not run
    }
    let source = "# Title\n\nSome **bold** text.\n\n```rust\nfn f() {}\n```\n";
    let (ed, bid) = markdown_editor(source);

    let layers = &ed.view.buffers[bid]
        .syntax
        .as_ref()
        .expect("engine syntax must be installed")
        .layers;
    assert!(
        layers.len() > 1,
        "expected root + at least one injected layer (bold text or rust fence), got {} layer(s)",
        layers.len()
    );
    assert_eq!(layers[0].depth, 0, "layers[0] must be the root layer");
    assert!(
        layers[1..].iter().all(|l| l.depth > 0),
        "every non-root layer must have depth > 0"
    );

    // At least one injected layer must be the rust fence (depth-1, non-empty
    // ranges strictly inside the buffer, root_node parses as rust).
    let has_rust_layer = layers[1..]
        .iter()
        .any(|l| l.tree.root_node().kind() == "source_file");
    assert!(
        has_rust_layer,
        "expected a rust-parsed layer for the fenced code block"
    );

    // At least one injected layer must be the markdown_inline layer for the
    // "**bold**"/"*italic*" text (markdown_inline's own root node kind is
    // also literally "inline"). This specifically exercises the outer
    // `(inline)` node's children-exclusion path in `content_ranges` — its
    // children (`strong_emphasis`, `emphasis`) are all *named*, so a bug
    // that excludes named children too (not just unnamed/punctuation ones)
    // yields empty ranges here and silently drops this whole layer, even
    // though the simpler rust-fence case above (a childless leaf node)
    // still passes.
    let has_inline_layer = layers[1..]
        .iter()
        .any(|l| l.tree.root_node().kind() == "inline");
    assert!(
        has_inline_layer,
        "expected a markdown_inline-parsed layer for the bold/italic text; got layer kinds: {:?}",
        layers[1..]
            .iter()
            .map(|l| l.tree.root_node().kind())
            .collect::<Vec<_>>()
    );
}

#[test]
fn bake_pending_edits_refreshes_injected_layer_ranges() {
    if !fixtures_present() {
        return;
    }
    let source = "```rust\nfn f() {}\n```\n";
    let (mut ed, bid) = markdown_editor(source);

    let ranges_before: Vec<_> = ed.view.buffers[bid]
        .syntax
        .as_ref()
        .unwrap()
        .layers
        .iter()
        .filter(|l| l.depth > 0)
        .map(|l| l.ranges.clone())
        .collect();
    assert!(
        !ranges_before.is_empty(),
        "expected at least one injected layer for the fenced code block"
    );

    // Insert 5 bytes at the very start of the buffer — every injected
    // layer's ranges (all strictly after byte 0) must shift by +5.
    ed.feed_key(key('i'));
    for ch in "XXXXX".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());

    // One reparse_stale_buffers call bakes pending edits (including the
    // range refresh) but does not yet install the queued precise reparse —
    // exactly the frame this test is about (see incremental_parse.rs's
    // `bake_aligns_committed_tree_before_precise_install`).
    ed.reparse_stale_buffers();

    let ranges_after: Vec<_> = ed.view.buffers[bid]
        .syntax
        .as_ref()
        .unwrap()
        .layers
        .iter()
        .filter(|l| l.depth > 0)
        .map(|l| l.ranges.clone())
        .collect();

    assert_eq!(
        ranges_before.len(),
        ranges_after.len(),
        "bake must not add or drop layers"
    );
    for (before, after) in ranges_before.iter().zip(ranges_after.iter()) {
        assert_eq!(
            before.len(),
            after.len(),
            "bake must not add or drop ranges within a layer"
        );
        for (b, a) in before.iter().zip(after.iter()) {
            assert_eq!(
                a.start_byte,
                b.start_byte + 5,
                "injected layer range must shift by the inserted byte count"
            );
            assert_eq!(a.end_byte, b.end_byte + 5);
        }
    }
}

#[test]
fn stale_gen_discards_whole_layer_set() {
    if !fixtures_present() {
        return;
    }
    let (mut ed, bid) = markdown_editor("```rust\nfn f() {}\n```\n");
    let gen0 = ed.state.buffers.get(bid).text_gen;

    // Construct a genuinely stale result: edit, let one reparse call post the
    // request (InlineParseBackend executes it and queues the result without
    // draining it), then edit *again* before the next drain — the queued
    // result now describes a superseded generation and must be discarded
    // whole (root + every injected layer), not partially applied.
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    let gen1 = ed.state.buffers.get(bid).text_gen;
    ed.reparse_stale_buffers(); // bakes, posts request for gen1; result queued

    ed.feed_key(key('i'));
    ed.feed_key(key('y'));
    ed.feed_key(key_esc());
    let gen2 = ed.state.buffers.get(bid).text_gen;
    assert!(
        gen2 > gen1,
        "premise: second edit must supersede the request"
    );

    ed.reparse_stale_buffers(); // drains the stale gen1 result; must discard

    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert_eq!(
        syn.parsed_gen, gen0,
        "stale result must be discarded whole — parsed_gen must stay at the \
         initial install, not advance to the stale request's generation"
    );

    // Recovery: the gen2 request (posted by the draining call above) installs
    // the full fresh layer set on the next drain.
    ed.reparse_stale_buffers();
    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert_eq!(syn.parsed_gen, gen2, "fresh result must install");
    let layers = &ed.view.buffers[bid].syntax.as_ref().unwrap().layers;
    assert!(
        layers.len() >= 2,
        "recovered layer set must include the rust fence layer, got {} layer(s)",
        layers.len()
    );
}

// ---------------------------------------------------------------------------
// PLUM pipeline syntax smoke test
// ---------------------------------------------------------------------------

/// Load the real `core:plum` plugin (the actual `runtime/plugins/core/plum/`
/// sources, not a synthetic copy) into `ed`, pointing `HUME_RUNTIME` at the
/// repo's real `runtime/` dir (so the real `grammar-sources.scm` catalog is
/// used) and `XDG_DATA_HOME` at `data_dir`. Env vars are process-global —
/// callers must hold `super::HUME_RUNTIME_MUTEX` for the test's duration.
#[cfg(not(windows))]
fn load_plum(ed: &mut Editor, data_dir: &std::path::Path) {
    let repo_runtime_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime");
    let config_tmp = tempfile::tempdir().unwrap();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), r#"(load-plugin "core:plum")"#).unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", &repo_runtime_dir);
        std::env::set_var("XDG_DATA_HOME", data_dir);
    }
    ed.init_scripting();
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }
}

/// `plum/register-installed-grammars!` runs its real body — including the
/// injections-path lookup — for every entry in the real `grammar-sources.scm`
/// catalog. None of them are compiled in the empty data dir, so every one is
/// skipped by the `when` guard and no network call happens; this is a pure
/// Scheme-syntax/logic smoke test for the PLUM changes, not an installation test.
#[test]
#[cfg(not(windows))]
fn plum_plugin_loads_with_real_grammar_catalog() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "loading core:plum against the real grammar-sources.scm must not error: {errors:?}"
    );
}

// ---------------------------------------------------------------------------
// plum-install-grammar / plum-update-grammar — optional name argument
// ---------------------------------------------------------------------------

/// `:plum-install-grammar` with no argument and no buffer language must warn
/// with the "no grammar name given" message — not the opaque install-failure
/// this used to log when the dead `(equal? name "")` guard let a `#f` name
/// fall through into the install pipeline.
#[test]
#[cfg(not(windows))]
fn plum_install_grammar_no_arg_no_language_warns() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar");

    let msgs: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("no grammar name given")
        }),
        "expected 'no grammar name given' warning, got: {msgs:?}"
    );
}

/// `:plum-install-grammar nosuchlang` — a name absent from the catalog warns
/// with the unknown-grammar message instead of failing deep inside the
/// install pipeline with an opaque hash-lookup error.
#[test]
#[cfg(not(windows))]
fn plum_install_grammar_unknown_name_warns() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");

    let msgs: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains(r#"unknown grammar "nosuchlang""#)
        }),
        "expected unknown-grammar warning naming 'nosuchlang', got: {msgs:?}"
    );
}

/// A typed argument wins over the current buffer's language.
///
/// Flip: before the arity-1 fix, `plum-install-grammar` was arity-0 so the
/// minibuffer silently dropped `nosuchlang` and the command installed `rust`
/// (the buffer's language) instead — verified by reverting the lambda to
/// arity-0, which made this test fail because it actually ran a real
/// `git-clone-rev`/`curl-fetch`/`compile-grammar!` install of `rust` instead
/// of ever mentioning `nosuchlang`.
#[test]
#[cfg(not(windows))]
fn plum_install_grammar_arg_overrides_buffer_language() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":set buffer language=rust");
    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");

    let msgs: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains(r#"unknown grammar "nosuchlang""#)
        }),
        "typed arg must win over buffer language 'rust', got: {msgs:?}"
    );
}

/// `:plum-update-grammar` gets the same optional-name treatment and the same
/// unknown-grammar validation — including that validation runs before the
/// stale-source `delete-dir` purge, so an unknown name deletes nothing.
#[test]
#[cfg(not(windows))]
fn plum_update_grammar_unknown_name_warns() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-update-grammar nosuchlang");

    let msgs: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains(r#"unknown grammar "nosuchlang""#)
        }),
        "expected unknown-grammar warning naming 'nosuchlang', got: {msgs:?}"
    );
}

/// `plum-install-grammar` is declared `#:inline-output #t` — dispatch must
/// only bracket it with the real terminal (alt-screen exit + "press any key
/// to return" block) when `Editor::run` owns the terminal. Off the event
/// loop (this test, like every other in this file, dispatches directly and
/// never calls `run`), that bracket must be skipped entirely: otherwise
/// dispatch blocks forever on a keypress that never comes whenever stdin
/// happens to be a real TTY (e.g. `cargo test` run interactively), which is
/// exactly what stalled the suite before `tui_active` was introduced.
#[test]
#[cfg(not(windows))]
fn inline_output_command_does_not_enter_terminal_bracket_off_event_loop() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    assert!(
        !ed.inline_output_entered(),
        "bracket must not have fired before any inline-output command ran"
    );

    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");

    assert!(
        !ed.inline_output_entered(),
        "inline-output bracket must stay skipped when Editor::run never took the terminal"
    );
}
