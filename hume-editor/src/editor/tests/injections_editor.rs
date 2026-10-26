// Editor-level tests for tree-sitter injection support: real markdown +
// markdown.inline + rust grammar fixtures, exercised through the full
// setup_buffer_syntax / reparse_stale_buffers / bake_pending_edits path.
//
// Requires scripts/fetch-test-grammars.sh (markdown, markdown.inline, rust).

use super::*;

use hume_scripting::ScriptingHost;
use hume_treesitter::registry::{OptionalQuery, QueryFiles};
use test_fixtures::{
    grammar_parser_path, grammar_query_path, helix_query_path, require_fixture_file,
    require_grammars, require_helix_query,
};

/// Give `name` its fixture grammar (source name == attach identity, true
/// for every real PLUM install; there is no renaming split in production),
/// registered only when `registered` lists it and attached otherwise.
/// `helix` lists the real Helix-maintained queries (what PLUM actually
/// installs) it gets besides its highlights.
fn give_grammar(
    ed: &mut Editor,
    name: &str,
    symbol: &str,
    helix: &[OptionalQuery],
    registered: &[&str],
) {
    let mut queries = QueryFiles::new(grammar_query_path(name));
    for &kind in helix {
        if let Some(path) = helix_query_path(name, kind.file_name()) {
            queries = queries.with(kind, path);
        }
    }
    if registered.contains(&name) {
        register_fixture_grammar_with(ed, name, symbol, &queries);
    } else {
        attach_fixture_grammar_with(ed, name, symbol, &queries);
    }
}

/// Require this file's grammar fixtures; see
/// `test_fixtures::require_grammars`/`require_fixture_file`.
fn require_fixtures() {
    require_grammars(&["markdown", "markdown.inline", "rust"]);
    let helix_path = grammar_parser_path("markdown").with_file_name("helix-injections.scm");
    require_fixture_file(&helix_path, "markdown helix-injections.scm");
}

/// Build an editor with markdown (+ the real Helix-maintained
/// `injections.scm`, what `:plum-install-grammar` actually fetches, not
/// the grammar's own bundled query), the `markdown.inline` grammar (the
/// name Helix's injections.scm resolves `(inline)` to, same name as the
/// fixture directory, no renaming needed), and rust, then attach markdown as
/// the buffer's language and drain the initial parse.
///
/// Builds the buffer directly (cursor at 0 via `Buffer::at_start`)
/// rather than through the `editor_from` marker DSL: this file's tests
/// mostly care about byte-offset edits at position 0, not cursor placement.
fn markdown_editor(source: &str) -> (Editor, hume_engine::pipeline::BufferId) {
    markdown_editor_with(source, &Dirs::none())
}

/// [`markdown_editor`] with a scripting host over `dirs`, so a language's
/// bundled data file is read from `dirs.runtime`.
fn markdown_editor_with(source: &str, dirs: &Dirs) -> (Editor, hume_engine::pipeline::BufferId) {
    markdown_editor_registering(source, dirs, &[])
}

/// [`markdown_editor_with`] where each grammar `registered` lists is
/// registered without loading, as startup does.
fn markdown_editor_registering(
    source: &str,
    dirs: &Dirs,
    registered: &[&str],
) -> (Editor, hume_engine::pipeline::BufferId) {
    markdown_editor_with_rust_queries(source, dirs, registered, &[])
}

/// [`markdown_editor_registering`] where rust also gets the Helix queries
/// `rust_helix` lists.
fn markdown_editor_with_rust_queries(
    source: &str,
    dirs: &Dirs,
    registered: &[&str],
    rust_helix: &[OptionalQuery],
) -> (Editor, hume_engine::pipeline::BufferId) {
    let buf = crate::editor::buffer::Buffer::at_start(hume_editing::text::BufferText::from(source));
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();

    ed.state
        .config
        .languages
        .register_identity("markdown", &["md"], &[], &[])
        .unwrap();
    give_grammar(
        &mut ed,
        "markdown",
        "tree_sitter_markdown",
        &[OptionalQuery::Injections],
        registered,
    );
    ed.state
        .config
        .languages
        .register_identity("markdown.inline", &[], &[], &[])
        .unwrap();
    give_grammar(
        &mut ed,
        "markdown.inline",
        "tree_sitter_markdown_inline",
        &[],
        registered,
    );
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    give_grammar(&mut ed, "rust", "tree_sitter_rust", rust_helix, registered);

    ed.scripting = Some(ScriptingHost::new(dirs));
    let lang = ed.state.config.languages.intern("markdown");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drains the initial full parse
    (ed, bid)
}

#[test]
fn markdown_buffer_installs_root_plus_injected_layers() {
    require_fixtures();
    let source = "# Title\n\nSome **bold** text.\n\n```rust\nfn f() {}\n```\n";
    let (ed, bid) = markdown_editor(source);

    let layers = &ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
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
    // `(inline)` node's children-exclusion path in `content_ranges`: its
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
    require_fixtures();
    let source = "```rust\nfn f() {}\n```\n";
    let (mut ed, bid) = markdown_editor(source);

    let ranges_before: Vec<_> = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
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

    // Insert 5 bytes at the very start of the buffer: every injected
    // layer's ranges (all strictly after byte 0) must shift by +5.
    ed.feed_key(key('i'));
    for ch in "XXXXX".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());

    // One reparse_stale_buffers call bakes pending edits (including the
    // range refresh) but does not yet install the queued precise reparse,
    // exactly the frame this test is about (see incremental_parse.rs's
    // `bake_aligns_committed_tree_before_precise_install`).
    ed.reparse_stale_buffers();

    let ranges_after: Vec<_> = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
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
    require_fixtures();
    let (mut ed, bid) = markdown_editor("```rust\nfn f() {}\n```\n");
    let gen0 = ed.state.buffers.get(bid).text().generation();

    // Construct a stale result: edit, let one reparse call post the
    // request (InlineParseBackend executes it and queues the result without
    // draining it), then edit *again* before the next drain. The queued
    // result now describes a superseded generation and must be discarded
    // whole (root + every injected layer), not partially applied.
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    let gen1 = ed.state.buffers.get(bid).text().generation();
    ed.reparse_stale_buffers(); // bakes, posts request for gen1; result queued

    ed.feed_key(key('i'));
    ed.feed_key(key('y'));
    ed.feed_key(key_esc());
    let gen2 = ed.state.buffers.get(bid).text().generation();
    assert!(
        gen2 > gen1,
        "premise: second edit must supersede the request"
    );

    ed.reparse_stale_buffers(); // drains the stale gen1 result; must discard

    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert_eq!(
        syn.parsed_gen(),
        Some(gen0),
        "stale result must be discarded whole: parsed_gen must stay at the \
         initial install, not advance to the stale request's generation"
    );

    // Recovery: the gen2 request (posted by the draining call above) installs
    // the full fresh layer set on the next drain.
    ed.reparse_stale_buffers();
    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert_eq!(syn.parsed_gen(), Some(gen2), "fresh result must install");
    let layers = &syn.layers().unwrap().layers;
    assert!(
        layers.len() >= 2,
        "recovered layer set must include the rust fence layer, got {} layer(s)",
        layers.len()
    );
}

fn bundled_loaded(ed: &Editor, name: &str) -> bool {
    let languages = &ed.state.config.languages;
    languages.bundled_loaded(languages.id_of(name).expect("a registered language"))
}

fn line_comment_primary(ed: &Editor, name: &str) -> Option<String> {
    let languages = &ed.state.config.languages;
    let id = languages.id_of(name).expect("a registered language");
    languages
        .comment_tokens_of(id)
        .and_then(|tokens| tokens.line())
        .map(|tokens| tokens.primary().to_owned())
}

#[test]
fn an_injected_layer_loads_its_languages_bundled_data() {
    require_fixtures();
    let (ed, bid) = markdown_editor_with("```rust\nfn f() {}\n```\n", &repo_runtime_dirs());

    assert!(
        has_layer_of(&ed, bid, "rust"),
        "premise: the fence is a rust layer"
    );
    assert!(bundled_loaded(&ed, "rust"));
    assert_eq!(line_comment_primary(&ed, "rust").as_deref(), Some("//"));
}

#[test]
fn an_attached_grammar_loads_its_languages_bundled_data_before_any_layer() {
    require_fixtures();
    let (ed, bid) = markdown_editor_with("# Title\n\nprose\n", &repo_runtime_dirs());

    assert!(!has_layer_of(&ed, bid, "rust"), "premise: no rust layer");
    assert!(bundled_loaded(&ed, "rust"));
    assert_eq!(line_comment_primary(&ed, "rust").as_deref(), Some("//"));
}

#[test]
fn a_registered_grammar_loads_its_languages_bundled_data_before_it_loads() {
    require_fixtures();
    let (ed, _bid) =
        markdown_editor_registering("# Title\n\nprose\n", &repo_runtime_dirs(), &["rust"]);
    let languages = &ed.state.config.languages;
    let rust = languages.id_of("rust").expect("rust is registered");

    assert!(
        languages.loaded_grammar(rust).is_none(),
        "premise: nothing needed rust"
    );
    assert!(bundled_loaded(&ed, "rust"));
    assert_eq!(line_comment_primary(&ed, "rust").as_deref(), Some("//"));
}

#[test]
fn a_fence_of_a_registered_grammar_gets_its_layer_without_rebuilding_syntax() {
    require_fixtures();
    let (mut ed, bid) =
        markdown_editor_registering("# T\n\n```rust\nfn f() {}\n```\n", &Dirs::none(), &["rust"]);
    let rust = ed
        .state
        .config
        .languages
        .id_of("rust")
        .expect("rust is registered");
    assert!(
        ed.state.config.languages.loaded_grammar(rust).is_some(),
        "the parse that met the fence loaded rust"
    );
    assert!(!has_layer_of(&ed, bid, "rust"));
    assert!(
        has_layers(&ed, bid),
        "the markdown layers stay while the fence resolves again"
    );

    ed.reparse_stale_buffers();

    assert!(has_layer_of(&ed, bid, "rust"));
}

#[test]
fn attaching_a_grammar_resolves_an_open_buffers_fences_without_rebuilding_syntax() {
    require_fixtures();
    require_grammars(&["json"]);
    let (mut ed, bid) = markdown_editor_with("# T\n\n```json\n[1]\n```\n", &Dirs::none());
    let json = attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");

    ed.sweep_buffers_for_grammars(vec![json.language]);

    assert!(has_layers(&ed, bid), "the markdown layers stay");
    ed.reparse_stale_buffers(); // posts the reparse
    ed.reparse_stale_buffers(); // installs it
    assert!(has_layer_of(&ed, bid, "json"));
}

/// `Editor::run` installs finished parses right before it dispatches a key,
/// so a fence grammar such a parse fails to load is reported before the key's
/// dispatch, not as a message of that key's own.
#[test]
fn a_fence_grammar_failing_in_a_parse_installed_before_a_key_is_not_its_message() {
    require_fixtures();
    let (mut ed, _bid) = markdown_editor("# T\n\n```json\n[1]\n```\n");
    let json = ed
        .state
        .config
        .languages
        .register_grammar(
            "json",
            GrammarSource::missing("json"),
            hume_origin::ConfigOrigin::User,
        )
        .expect("a user grammar replaces any other");
    ed.sweep_buffers_for_grammars(vec![json]);
    ed.reparse_stale_buffers(); // posts the reparse; its result waits

    ed.install_parse_results();
    ed.handle_input(TerminalEvent::Key(key('l')));

    assert_eq!(
        super::scripting_grammar::grammar_warnings(&ed, "json").len(),
        1
    );
    assert!(!ed.state.message_logged_this_input);
}

#[test]
fn a_fence_grammar_that_fails_to_load_warns_once_and_keeps_the_layers() {
    require_fixtures();
    let (mut ed, bid) = markdown_editor("# T\n\n```json\n[1]\n```\n");
    let json = ed
        .state
        .config
        .languages
        .register_grammar(
            "json",
            GrammarSource::missing("json"),
            hume_origin::ConfigOrigin::User,
        )
        .expect("a user grammar replaces any other");
    ed.sweep_buffers_for_grammars(vec![json]);
    for _ in 0..3 {
        ed.reparse_stale_buffers();
    }
    type_text(&mut ed, "x");
    ed.reparse_stale_buffers(); // posts the reparse of the edit
    ed.reparse_stale_buffers(); // installs it

    assert_eq!(
        super::scripting_grammar::grammar_warnings(&ed, "json").len(),
        1
    );
    assert!(!ed.state.config.languages.has_grammar("json"));
    assert!(has_layers(&ed, bid), "the markdown layers stay");
    assert!(!has_layer_of(&ed, bid, "json"));
}

/// Right after the parse that loads a fence's grammar, no committed layer
/// has a textobjects query, yet a text object in the fence finds the
/// fence's function.
#[test]
fn a_text_object_in_a_fence_whose_grammar_just_loaded_finds_its_function() {
    require_fixtures();
    require_helix_query("rust", OptionalQuery::TextObjects.file_name());
    let (mut ed, bid) = markdown_editor_with_rust_queries(
        "```rust\nfn f() {\n    1\n}\n```\n",
        &Dirs::none(),
        &["rust"],
        &[OptionalQuery::TextObjects],
    );
    let syn = ed.state.buffers.get(bid).syntax.as_ref().expect("syntax");
    assert!(
        syn.reresolve_pending(),
        "premise: rust loaded, the fence not yet resolved again"
    );
    assert!(!has_layer_of(&ed, bid, "rust"));

    ed.feed_keys([key('j'), key('j'), key('m'), key('i'), key('f')]);

    assert_eq!(
        ed.current_view().primary().slice().to_string(),
        "{\n    1\n}"
    );
}

#[test]
fn a_fence_of_a_registered_grammar_typed_just_before_toggling_takes_its_comment() {
    require_fixtures();
    let (mut ed, bid) = markdown_editor_registering("x\n", &repo_runtime_dirs(), &["rust"]);
    type_text(&mut ed, "~~~rust\nf\n~~~\n");
    ed.feed_keys([key(';'), key('k')]);

    ed.feed_keys([key('G'), key('G')]);

    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "~~~rust\n// f\n~~~\nx\n"
    );
}

#[test]
fn a_fence_typed_just_before_toggling_takes_its_languages_comment() {
    require_fixtures();
    let (mut ed, bid) = markdown_editor_with("x\n", &repo_runtime_dirs());
    type_text(&mut ed, "~~~rust\nf\n~~~\n");
    ed.feed_keys([key(';'), key('k')]);

    ed.feed_keys([key('G'), key('G')]);

    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "~~~rust\n// f\n~~~\nx\n"
    );
}

#[test]
fn a_cursor_in_a_rust_fence_takes_rusts_line_comment() {
    require_fixtures();
    let (mut ed, bid) =
        markdown_editor_with("# T\n\n```rust\nfn f() {}\n```\n", &repo_runtime_dirs());
    ed.feed_keys([key('j'), key('j'), key('j')]);

    ed.feed_keys([key('G'), key('G')]);

    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "# T\n\n```rust\n// fn f() {}\n```\n"
    );
}

#[test]
fn a_cursor_on_a_paragraph_takes_markdowns_block_comment() {
    require_fixtures();
    let (mut ed, bid) = markdown_editor_with("some *text*\n", &repo_runtime_dirs());

    ed.feed_keys([key('G'), key('G')]);

    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "<!-- some *text* -->\n"
    );
}

#[test]
fn a_selection_crossing_a_fence_takes_the_outer_language() {
    require_fixtures();
    let (mut ed, bid) = markdown_editor_with("```rust\nfn f() {}\n```\n", &repo_runtime_dirs());
    ed.state.input.set_extend(true);
    ed.feed_key(key('j'));

    ed.feed_keys([key('G'), key('G')]);

    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "<!-- ```rust\nfn f() {} -->\n```\n"
    );
}

/// Types inside the first of two rust fences, then brings the layers
/// current either by the background path (`reparse_stale_buffers` over the
/// inline backend) or the foreground one (`ensure_syntax_current`).
fn fence_edited_editor(foreground: bool) -> (Editor, hume_engine::pipeline::BufferId) {
    let source = "# T\n\n```rust\nfn a() {}\n```\n\nText.\n\n```rust\nfn b() {}\n```\n";
    let (mut ed, bid) = markdown_editor(source);
    ed.feed_keys([key('j'), key('j'), key('j')]);
    type_text(&mut ed, "let y = 1; ");
    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "# T\n\n```rust\nlet y = 1; fn a() {}\n```\n\nText.\n\n```rust\nfn b() {}\n```\n"
    );
    if foreground {
        crate::editor::syntax::ensure_syntax_current(
            &mut ed.state,
            &mut ed.view.registry,
            bid,
            crate::editor::syntax::SyntaxUse::InjectedLanguages,
        );
    } else {
        ed.reparse_stale_buffers(); // bakes and posts
        ed.reparse_stale_buffers(); // installs
    }
    (ed, bid)
}

fn installed_shapes(
    ed: &Editor,
    bid: hume_engine::pipeline::BufferId,
) -> Vec<hume_treesitter::layers::LayerShape> {
    let buf = ed.state.buffers.get(bid);
    let syn = buf.syntax.as_ref().unwrap();
    assert!(
        syn.is_current(buf.text().generation()),
        "the installed layers describe the edited text"
    );
    syn.layers().unwrap().shapes()
}

fn injected_afresh(
    ed: &Editor,
    bid: hume_engine::pipeline::BufferId,
) -> Vec<hume_treesitter::layers::LayerShape> {
    let buf = ed.state.buffers.get(bid);
    buf.syntax
        .as_ref()
        .unwrap()
        .injected_shapes_resolved_afresh(buf.text(), &ed.state.config.languages.grammar_snapshot())
}

fn rust_fence_trees_changed(ed: &Editor, bid: hume_engine::pipeline::BufferId) -> Vec<bool> {
    let rust = ed.state.config.languages.id_of("rust");
    ed.state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
        .unwrap()
        .layers
        .iter()
        .filter(|l| Some(l.bundle.language) == rust)
        .map(|l| l.tree.root_node().has_changes())
        .collect()
}

#[test]
fn background_and_foreground_reparse_of_a_fence_edit_agree_with_a_fresh_resolution() {
    require_fixtures();
    let (background, background_bid) = fence_edited_editor(false);
    let (foreground, foreground_bid) = fence_edited_editor(true);

    let background_shapes = installed_shapes(&background, background_bid);
    let foreground_shapes = installed_shapes(&foreground, foreground_bid);
    // Each editor owns its grammar bundles, so the layers compare by language.
    assert_eq!(background_shapes.len(), foreground_shapes.len());
    for (b, f) in background_shapes.iter().zip(&foreground_shapes) {
        assert_eq!(b.bundle.language, f.bundle.language);
        assert_eq!(b.depth, f.depth);
        assert_eq!(b.ranges, f.ranges);
        assert_eq!(b.nodes, f.nodes);
    }

    let afresh = injected_afresh(&background, background_bid);
    assert!(
        !afresh.is_empty(),
        "premise: the fences are injected layers"
    );
    assert_eq!(
        hume_treesitter::layers::first_layer_mismatch(&afresh, &background_shapes[1..]),
        None,
        "the installed injected layers equal a resolution with no prior"
    );
    assert_eq!(
        hume_treesitter::layers::first_layer_mismatch(
            &injected_afresh(&foreground, foreground_bid),
            &foreground_shapes[1..]
        ),
        None,
        "the synchronously installed injected layers equal a resolution with no prior"
    );
}

#[test]
fn an_edit_in_one_fence_reuses_the_other_fences_tree_on_both_paths() {
    require_fixtures();
    for foreground in [false, true] {
        let (ed, bid) = fence_edited_editor(foreground);

        assert_eq!(
            rust_fence_trees_changed(&ed, bid),
            [false, true],
            "foreground={foreground}: the edited fence reparses, the later fence keeps its baked tree"
        );
    }
}

// ---------------------------------------------------------------------------
// PLUM pipeline syntax smoke test
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// plum-install-grammar — optional name argument
// ---------------------------------------------------------------------------
