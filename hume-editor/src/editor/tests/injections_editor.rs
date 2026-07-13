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

/// `:plum-list` exercises `plugins.scm`'s post-migration `plum/installed-plugins`
/// (now built on `plum/list-dir`, a Steel `read-dir`-backed helper, instead of
/// the removed `list-dir` builtin) against a real (empty) data dir — no
/// network. Pins that the migration to Steel's stdlib process/fs helpers
/// (see docs/ROADMAP.md's plugin trust model decision) didn't break loading
/// or basic discovery.
#[test]
#[cfg(not(windows))]
fn plum_list_runs_with_no_errors_against_empty_data_dir() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-list");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        ":plum-list against an empty data dir must not error: {errors:?}"
    );
}

/// Run `git` with `args` in `dir`, asserting success — test-setup helper
/// only (builds local origin/clone fixtures), not part of the migration
/// under test.
#[cfg(not(windows))]
fn git_ok(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} in {dir:?} failed");
}

/// `:plum-update` exercises `plum/run!` (Phase 1 helper, now backing
/// `git-pull`'s replacement) against a REAL local git repo — no network. A
/// local "origin" gets a second commit after the "installed" clone is made,
/// then `:plum-update` must actually run `git pull` (via Steel's
/// `spawn-process` + `with-current-dir`, not the removed `git-pull`
/// builtin) and fast-forward the clone to match.
#[test]
#[cfg(not(windows))]
fn plum_update_runs_real_git_pull_against_local_origin() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let origin_tmp = tempfile::tempdir().unwrap();
    let origin_dir = origin_tmp.path();
    git_ok(origin_dir, &["init", "-q"]);
    git_ok(origin_dir, &["config", "user.email", "test@example.com"]);
    git_ok(origin_dir, &["config", "user.name", "Test"]);
    std::fs::write(origin_dir.join("plugin.scm"), "; v1\n").unwrap();
    git_ok(origin_dir, &["add", "plugin.scm"]);
    git_ok(origin_dir, &["commit", "-q", "-m", "v1"]);

    let data_tmp = tempfile::tempdir().unwrap();
    let clone_dir = data_tmp.path().join("hume/plugins/testuser/testrepo");
    std::fs::create_dir_all(clone_dir.parent().unwrap()).unwrap();
    git_ok(
        data_tmp.path(),
        &[
            "clone",
            "-q",
            origin_dir.to_str().unwrap(),
            clone_dir.to_str().unwrap(),
        ],
    );

    // Advance the origin past what the clone has.
    std::fs::write(origin_dir.join("plugin.scm"), "; v2\n").unwrap();
    git_ok(origin_dir, &["add", "plugin.scm"]);
    git_ok(origin_dir, &["commit", "-q", "-m", "v2"]);

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-update");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        ":plum-update against a local origin must not error: {errors:?}"
    );
    let content = std::fs::read_to_string(clone_dir.join("plugin.scm")).unwrap();
    assert_eq!(
        content, "; v2\n",
        "plum/run!-backed git pull must fast-forward the clone to origin's latest commit"
    );
}

/// `:plum-cleanup` exercises `plum/delete-dir` (Phase 1 helper, now backing
/// `delete-dir`'s replacement) against a real on-disk orphan plugin — no
/// network. Nothing in `init.scm` declares it, so it's an orphan by
/// definition; `:plum-cleanup` must remove its directory.
#[test]
#[cfg(not(windows))]
fn plum_cleanup_removes_orphan_plugin_directory() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let orphan_dir = data_tmp.path().join("hume/plugins/testuser/orphanrepo");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    std::fs::write(orphan_dir.join("plugin.scm"), "; orphan\n").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-cleanup");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        ":plum-cleanup must not error: {errors:?}"
    );
    assert!(
        !orphan_dir.exists(),
        "plum/delete-dir-backed plum-cleanup must remove the orphan plugin directory"
    );
}

// ---------------------------------------------------------------------------
// plum-install-grammar — optional name argument
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
/// install pipeline with an opaque hash-lookup error. This validation runs
/// before the stale-source `delete-dir` purge in `plum/install-grammar`, so
/// an unknown name deletes nothing.
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

/// `plum-install-grammar` is declared `#:inline-output #t` — dispatch must
/// only bracket it with the real terminal (alt-screen exit + "press any key
/// to return" block) when `Editor::run` owns the terminal. Off the event
/// loop (this test, like every other in this file, dispatches directly and
/// never calls `run`), that bracket must be skipped entirely: otherwise
/// dispatch blocks forever on a keypress that never comes whenever stdin
/// happens to be a real TTY (e.g. `cargo test` run interactively), which is
/// exactly what stalled the suite before `tui_active` was introduced.
///
/// This particular command errors out via `log!` only (no `displayln`), so
/// under the lazy-entry design it never even reaches
/// `ensure_inline_output_screen` — see
/// `inline_output_command_with_real_output_still_skips_bracket_off_event_loop`
/// below for the case that does.
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

/// `lsp-servers` is `#:inline-output #t` and *does* print via `displayln`
/// (one line per seeded server) — off the event loop that must still reach
/// `EditorHostImpl::ensure_inline_output_screen`'s `Headless` no-op branch
/// rather than the real terminal: printing must succeed without ever
/// flipping `inline_output_entered()`.
///
/// Flip: hardcode `ensure_inline_output_screen` to always enter (drop the
/// `Headless`/`Armed` distinction) → this test hangs on `wait_for_keypress`
/// against a real TTY, or panics against a non-TTY stdin in CI.
#[test]
#[cfg(not(windows))]
fn inline_output_command_with_real_output_still_skips_bracket_off_event_loop() {
    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-servers");

    assert!(
        !ed.inline_output_entered(),
        "displayln output off the event loop must not enter the real terminal bracket"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("seeded servers")),
        "lsp-servers must still log its summary line, got: {:?}",
        ed.state.status_msg
    );
}

/// Regression test: a grammar's source dir left non-empty by a prior failed
/// install (clone succeeded, compile didn't) must not break
/// `:plum-install-grammar` on the very next attempt — `plum/install-grammar`
/// purges any existing source dir before re-cloning, so retrying "just works"
/// on the first try instead of requiring a second attempt to clear the
/// leftover directory as a side effect of the first attempt's own failure.
///
/// Hits the network (real git clone + curl fetch + tree-sitter build of the
/// `json` grammar); gated like `install_real_json_grammar_e2e` in
/// `scripting_grammar.rs`.
///
/// Flip: without the `(delete-dir src-dir)` fix in `plum/install-grammar`,
/// `git-clone-rev` refuses to clone into this pre-seeded non-empty dir and
/// the command logs an error instead of installing — `out_path` never
/// appears, and this assertion fails.
#[test]
#[cfg(not(windows))]
fn plum_install_grammar_recovers_from_stale_source_dir_on_first_try() {
    use std::process::Command;

    let require_live = std::env::var("HUME_REQUIRE_LIVE_GRAMMAR_E2E")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !require_live {
        eprintln!(
            "plum_install_grammar_recovers_from_stale_source_dir_on_first_try: skipping \
             (set HUME_REQUIRE_LIVE_GRAMMAR_E2E=1 to run live e2e)"
        );
        return;
    }
    let has_git = Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let has_curl = Command::new("curl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let has_ts = Command::new("tree-sitter")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_git || !has_curl || !has_ts {
        panic!(
            "HUME_REQUIRE_LIVE_GRAMMAR_E2E=1 but git={has_git} curl={has_curl} tree-sitter={has_ts}"
        );
    }

    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    // `load_plum` points XDG_DATA_HOME at data_tmp — the real data dir is
    // XDG_DATA_HOME/hume (see sandbox.rs's init_dirs).
    let data_dir = data_tmp.path().join("hume");
    // Seed a stale, non-empty source dir exactly like a prior clone-succeeded/
    // compile-failed install would leave behind — git-clone-rev refuses to
    // clone into this without the pre-clean fix.
    let src_dir = data_dir.join("grammars/sources/json");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("stale.txt"), b"leftover from a failed install").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar json");

    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let out_path = data_dir.join("grammars").join(format!("json.{ext}"));
    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .map(|e| format!("{:?}: {}", e.severity, e.text))
        .collect();
    assert!(
        out_path.exists(),
        "compiled json grammar must exist after install recovers from a stale \
         source dir on the first try; log={errors:#?}"
    );
    assert!(
        !src_dir.join("stale.txt").exists(),
        "stale leftover file must be purged by the pre-clone delete-dir"
    );
}

/// `tsx`'s `highlights.scm` declares `; inherits: ecma,_typescript,_jsx`
/// instead of writing out its own patterns — `plum/install-grammar` must
/// resolve that chain (`plum/resolve-query` in `grammars.scm`) so the file
/// written to disk has real capture patterns, not a dangling directive.
///
/// Hits the network (real git clone + tree-sitter build of the `tsx` grammar,
/// plus curl fetches of its `highlights.scm` and its `ecma`/`_typescript`/
/// `_jsx` query dependencies); gated like `install_real_json_grammar_e2e`.
///
/// Flip: reverting the `plum/fetch-query!` call sites back to plain
/// `curl-fetch` leaves `highlights.scm` as the raw `; inherits: …` stub —
/// the `starts_with("; inherits")` and `contains('@')` assertions below both
/// fail on that stub (no `@capture` in a comment-only file).
#[test]
#[cfg(not(windows))]
fn plum_install_grammar_resolves_helix_inherits_chain() {
    let require_live = std::env::var("HUME_REQUIRE_LIVE_GRAMMAR_E2E")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !require_live {
        eprintln!(
            "plum_install_grammar_resolves_helix_inherits_chain: skipping \
             (set HUME_REQUIRE_LIVE_GRAMMAR_E2E=1 to run live e2e)"
        );
        return;
    }
    use std::process::Command;
    let has_git = Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let has_curl = Command::new("curl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let has_ts = Command::new("tree-sitter")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_git || !has_curl || !has_ts {
        panic!(
            "HUME_REQUIRE_LIVE_GRAMMAR_E2E=1 but git={has_git} curl={has_curl} tree-sitter={has_ts}"
        );
    }

    let _lock = super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let data_tmp = tempfile::tempdir().unwrap();
    let data_dir = data_tmp.path().join("hume");

    let buf = crate::editor::buffer::Buffer::new(
        hume_editing::text::Text::from("const x: number = 1;\n"),
        hume_editing::selection::SelectionSet::default(),
    );
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();
    load_plum(&mut ed, data_tmp.path());
    // Real bootstrap loads runtime/scheme/languages.scm (which declares tsx's
    // identity) before any plugin runs; `load_plum` only loads `core:plum`,
    // so register the identity here to match that ordering — `register-grammar!`
    // attaches onto an existing identity, it doesn't create one.
    ed.state
        .languages
        .register_identity("tsx", &["tsx"], &[], &[])
        .unwrap();

    type_cmd(&mut ed, ":plum-install-grammar tsx");

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .map(|e| format!("{:?}: {}", e.severity, e.text))
        .collect();

    let hl_path = data_dir.join("grammars/sources/tsx/highlights.scm");
    let hl_content = std::fs::read_to_string(&hl_path).unwrap_or_else(|e| {
        panic!("highlights.scm must exist after install; log={errors:#?}; err={e}")
    });
    assert!(
        !hl_content.trim_start().starts_with("; inherits"),
        "highlights.scm must be resolved, not left as a dangling inherits stub: {hl_content:?}"
    );
    assert!(
        hl_content.contains('@'),
        "resolved highlights.scm must contain real tree-sitter capture patterns, got: {hl_content:?}"
    );

    ed.set_buffer_language(bid, Some("tsx".to_owned()));
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must attach after tsx grammar install; log={errors:#?}"
    );
}
