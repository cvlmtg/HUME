//! `core:buffer-words` — end-to-end plugin tests. Loads the real, shipped
//! `runtime/plugins/core/buffer-words/plugin.scm` (`HumeRuntimeGuard` +
//! `write_core_plugin`, the pattern `pickers_plugin.rs` documents), except
//! the last test, which needs the multi-file `core:lsp` too and so uses
//! `RealRuntimeGuard` instead (`git_diff_plugin.rs`'s reason for the same
//! choice — see its module doc).
//!
//! Independent oracle throughout: every expected word set below is written
//! by hand from the fixture's own text, never produced by the plugin's own
//! scan.

use super::*;

use std::path::Path;
use std::time::{Duration, Instant};

use hume_scripting::ScriptingHost;

const BUFFER_WORDS_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/buffer-words/plugin.scm"
));

/// Stages the real `core:buffer-words` (and its `core:stdlib` dependency)
/// into `guard`'s isolated runtime and loads them — no buffer is open yet:
/// every test opens its own fixture afterward, via [`open`], so
/// `on-buffer-open` fires with the plugin's hook already registered
/// (`Editor::open`'s own startup buffer opens *before* any plugin loads,
/// same as `git_diff_plugin.rs`'s `setup`/`open` split).
fn setup(guard: &HumeRuntimeGuard, tmp: &Path, config_expr: Option<&str>) -> Editor {
    write_core_plugin(guard, "buffer-words", BUFFER_WORDS_PLUGIN);
    write_core_plugin(guard, "stdlib", STDLIB_PLUGIN);
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let mut host = ScriptingHost::new();
    let load_bw = match config_expr {
        Some(cfg) => format!("(load-plugin \"core:buffer-words\" #:config {cfg})"),
        None => "(load-plugin \"core:buffer-words\")".to_string(),
    };
    let source = format!("(load-plugin \"core:stdlib\")\n{load_bw}");
    eval_with_real_host(&mut ed, &mut host, &source, tmp);
    ed.scripting = Some(host);
    ed
}

/// Opens `path` as a real, file-backed buffer (mirrors `git_diff_plugin.rs`'s
/// `open`) and returns its id.
fn open(ed: &mut Editor, path: &Path) -> BufferId {
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    ed.focused_buffer_id()
}

/// The ranked labels the open completion session would show, top 20 — a
/// local copy of `tests/completion/mod.rs`'s `labels`, since that one is
/// private to a sibling module tree.
fn labels(ed: &Editor) -> Vec<String> {
    ed.state
        .input
        .completion()
        .map(|s| {
            s.top(20, &ed.state.config.completion_sources)
                .iter()
                .map(|v| v["label"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Ctrl-Space, then settle so the queued Steel source answers.
fn trigger(ed: &mut Editor) {
    ed.feed_key(key_ctrl(' '));
    ed.settle();
}

/// Types `text` char by char without entering or leaving Insert mode —
/// unlike `tests/mod.rs`'s `type_text`, which brackets its typing with its
/// own `i`/Escape and so always ends back in Normal mode. Every test here
/// needs to stay in Insert for `trigger`'s Ctrl-Space (gated on Insert mode)
/// to do anything at all, so the caller enters Insert itself first.
fn type_in_insert(ed: &mut Editor, text: &str) {
    for ch in text.chars() {
        ed.feed_key(key(ch));
    }
}

/// Waits (bounded, 2s) for `target` to appear among the ranked labels,
/// re-triggering completion each attempt. The background index's own state
/// lives entirely in Steel with no clean seam to observe directly from
/// Rust (the same constraint `git_diff_plugin.rs`'s module doc describes
/// for that plugin's fetch state), so this drives the same observable
/// behaviour a user would: press Ctrl-Space, see what's offered, and if the
/// word being waited on isn't there yet, let the background walk advance
/// (`trigger`'s own `settle()`) and try again.
fn wait_for_word(ed: &mut Editor, target: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        trigger(ed);
        if labels(ed).iter().any(|l| l == target) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{target:?} never appeared in the candidate list"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// [`wait_for_word`]'s negation, for the "an edit removed this word" half of
/// the refresh test.
fn wait_for_word_gone(ed: &mut Editor, target: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        trigger(ed);
        if !labels(ed).iter().any(|l| l == target) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{target:?} never dropped out of the candidate list"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn ctrl_space_offers_identifiers_from_the_buffer() {
    let tmp = safe_tempdir();
    let guard = HumeRuntimeGuard::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "alpha beta\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    trigger(&mut ed);
    let got = labels(&ed);
    assert!(got.contains(&"alpha".to_string()), "{got:?}");
    assert!(got.contains(&"beta".to_string()), "{got:?}");
}

/// A word far from the cursor, on a buffer with many more lines than one
/// tick's `"lines"` budget, is still found — the actual case a background
/// walk exists for. Not a red/green oracle for the line-windowed rewrite:
/// the pre-rewrite whole-buffer read would also eventually find this word
/// (same total content, just read eagerly rather than incrementally), so
/// there is no meaningful pre-rewrite baseline to diverge from here. This is
/// a regression/characterization test for `bw/walk!`'s per-tick fetch
/// arithmetic instead: an off-by-one in `fwd-hi`/`bwd-lo` would skip the
/// line this fixture puts the target word on, rather than merely being
/// masked by overlap with an already-covered line.
#[test]
fn a_word_many_lines_past_the_cursor_is_offered() {
    let tmp = safe_tempdir();
    let guard = HumeRuntimeGuard::new();
    let mut ed = setup(&guard, tmp.path(), Some(r#"(hash "lines" 20)"#));
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    let mut content = "filler\n".repeat(300);
    content.push_str("target_word\n");
    std::fs::write(&path, &content).unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    wait_for_word(&mut ed, "target_word");
}

#[test]
fn the_partially_typed_token_is_not_offered_back() {
    let tmp = safe_tempdir();
    let guard = HumeRuntimeGuard::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "hello helloworld\n\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key_down());
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "hello");
    trigger(&mut ed);
    let got = labels(&ed);
    assert!(!got.contains(&"hello".to_string()), "{got:?}");
    // Not a zero-effect check on its own: an empty menu would vacuously
    // satisfy the assertion above without proving anything. "helloworld"
    // also starts with the live prefix "hello" (so Rust's own re-ranking
    // doesn't filter it out the way it would something unrelated), but
    // isn't *equal* to it — pinning that the exclusion is specifically
    // "equals the prefix", not an accident of the prefix filter itself.
    assert!(got.contains(&"helloworld".to_string()), "{got:?}");
}

/// Emit-all's reason for existing: a Steel-side prefix filter pinned at
/// trigger time could never widen back out on Backspace, since the source
/// answers once and isn't re-invoked. This only passes if the *whole*
/// cached set was emitted up front and Rust's own re-ranking (`session.rs`'s
/// `rank`) is what narrows/widens it per keystroke.
#[test]
fn backspace_widens_the_candidate_list_again() {
    let tmp = safe_tempdir();
    let guard = HumeRuntimeGuard::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "cat catalog\n\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key_down());
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "catal");
    trigger(&mut ed);
    let narrow = labels(&ed);
    assert!(narrow.contains(&"catalog".to_string()), "{narrow:?}");
    assert!(!narrow.contains(&"cat".to_string()), "{narrow:?}");

    ed.feed_key(key_backspace());
    ed.feed_key(key_backspace());
    ed.settle();
    let widened = labels(&ed);
    assert!(widened.contains(&"cat".to_string()), "{widened:?}");
    assert!(widened.contains(&"catalog".to_string()), "{widened:?}");
}

#[test]
fn an_edit_refreshes_the_index() {
    let tmp = safe_tempdir();
    let guard = HumeRuntimeGuard::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    // A trailing space keeps "newword" out of the live token, so it isn't
    // excluded as the in-progress prefix (see `the_partially_typed_token_…`
    // above) — this test is about the refresh, not the exclusion.
    let typed = "newword ";
    type_in_insert(&mut ed, typed);
    wait_for_word(&mut ed, "newword");

    for _ in 0..typed.chars().count() {
        ed.feed_key(key_backspace());
    }
    wait_for_word_gone(&mut ed, "newword");
}

#[test]
fn fuzzy_match_is_opt_in() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "roundtrip\n\n").unwrap();

    let guard = HumeRuntimeGuard::new();
    let mut ed = setup(&guard, tmp.path(), None);
    open(&mut ed, &path);
    ed.feed_key(key_down());
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "rtp");
    trigger(&mut ed);
    assert!(
        !labels(&ed).contains(&"roundtrip".to_string()),
        "default 'string match must reject a subsequence-only candidate"
    );
    drop(ed);
    drop(guard);

    let tmp2 = safe_tempdir();
    let guard2 = HumeRuntimeGuard::new();
    let mut ed2 = setup(&guard2, tmp2.path(), Some(r#"(hash "match" 'fuzzy)"#));
    open(&mut ed2, &path);
    ed2.feed_key(key_down());
    ed2.feed_key(key('i'));
    type_in_insert(&mut ed2, "rtp");
    trigger(&mut ed2);
    assert!(
        labels(&ed2).contains(&"roundtrip".to_string()),
        "#:config (hash \"match\" 'fuzzy) must accept a subsequence match"
    );
}

#[test]
fn word_chars_extends_what_counts_as_a_word() {
    let tmp = safe_tempdir();
    let guard = HumeRuntimeGuard::new();
    let mut ed = setup(&guard, tmp.path(), None);
    // Set the *global* default before opening the fixture, rather than a
    // buffer-scoped override after: the buffer this plugin cares about
    // doesn't exist yet, and setting it first means the very first index —
    // built synchronously by `on-buffer-open` — already sees it, with no
    // need to force a second reindex.
    type_cmd(&mut ed, ":set global word-chars=-");
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "foo-bar\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(
        labels(&ed).contains(&"foo-bar".to_string()),
        "{:?}",
        labels(&ed)
    );
}

#[test]
fn an_invalid_match_config_fails_the_load() {
    use hume_scripting::PluginStatus;
    use hume_scripting::attribution::PluginId;

    let tmp = safe_tempdir();
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "buffer-words", BUFFER_WORDS_PLUGIN);
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);

    let init_path = tmp.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:buffer-words\" #:config (hash \"match\" 'regex))",
    )
    .unwrap();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let mut host = ScriptingHost::new();
    let err = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect_err("an unlisted \"match\" value must fail eval_init");

    assert!(
        err.message.contains("core:buffer-words") && err.message.contains("must be one of"),
        "error must carry the plugin's own prefixed message; got {:?}",
        err.message
    );

    ed.apply_script_effects(err.effects);
    ed.scripting = Some(host);

    let id = PluginId::Core("buffer-words".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "core:buffer-words must be marked Failed after its body raises"
    );
}

/// Closing a buffer mid-walk must cancel its timer chain rather than leave
/// an orphaned continuation running against a since-replaced (or absent)
/// entry — there's no clean seam to assert the entry is actually gone from
/// Steel state, so this observes the property that matters from Rust: the
/// orphaned continuation, if the cancel didn't happen, would eventually
/// fire and either panic or write into whatever the same buffer id's entry
/// becomes next. Draining well past when the walk would have finished, with
/// no panic, is the achievable proxy for "no leak."
#[test]
fn closing_a_buffer_drops_its_index() {
    let tmp = safe_tempdir();
    let guard = HumeRuntimeGuard::new();
    let mut ed = setup(&guard, tmp.path(), Some(r#"(hash "lines" 1)"#));
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    let content = "filler\n".repeat(20);
    std::fs::write(&path, &content).unwrap();
    open(&mut ed, &path);
    ed.execute_typed("bd", None).unwrap();
    for _ in 0..20 {
        ed.settle();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The plan's reason for existing: `core:buffer-words` and `core:lsp`
/// sharing one buffer, exercising multi-source ranking (`#:priority`) with
/// two real, independently-motivated sources — nothing else in the
/// codebase does this today. `RealRuntimeGuard` rather than
/// `HumeRuntimeGuard`: `core:lsp` is multi-file (see `git_diff_plugin.rs`'s
/// module doc for the same reasoning), and it picks up this plugin's own
/// real, just-shipped `runtime/plugins/core/buffer-words/plugin.scm` too —
/// no separate staging needed.
#[test]
fn buffer_words_and_lsp_rank_together_lsp_first() {
    use hume_lsp::backend::ServerId;
    use hume_lsp::client::LspClient;
    use hume_lsp::test_util::RecordingLspBackend;

    let tmp = safe_tempdir();
    let _guard = RealRuntimeGuard::new();

    let (mut backend, _notifications, requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({ "capabilities": { "completionProvider": {} } }),
    );
    backend.respond_to(
        "textDocument/completion",
        serde_json::json!([{"label": "lsp_item"}]),
    );
    let sid: ServerId = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = crate::editor::lsp::LspState::from_backend_for_test(Box::new(backend));

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:buffer-words\")\n(load-plugin \"core:lsp\")",
        tmp.path(),
    );
    ed.scripting = Some(host);

    let file_dir = safe_tempdir();
    let path = file_dir.path().join("main.rs");
    std::fs::write(&path, "buffer_word\n").unwrap();
    let bid = open(&mut ed, &path);
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    let lang = ed.state.config.languages.intern("rust");
    ed.state.buffers.get_mut(bid).language = Some(lang);

    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), std::path::PathBuf::from("."), sid);
    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    ed.settle();

    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.drain_lsp();
    ed.settle();

    let got = labels(&ed);
    assert!(
        got.iter().any(|l| l == "buffer_word"),
        "buffer-words' own candidate missing: {got:?}"
    );
    assert!(
        got.iter().any(|l| l == "lsp_item"),
        "core:lsp's candidate missing: {got:?}"
    );
    assert_eq!(
        got.iter().position(|l| l == "lsp_item"),
        Some(0),
        "core:lsp's higher #:priority must rank it first: {got:?}"
    );
    assert!(request_count(&requests, "textDocument/completion") >= 1);
}
