//! `core:buffer-words`: end-to-end plugin tests. Loads the real, shipped
//! `runtime/plugins/core/buffer-words/plugin.scm` (`RuntimeDirs` +
//! `write_core_plugin`, the pattern `pickers_plugin.rs` documents), except
//! the last test, which needs the multi-file `core:lsp` too and so uses
//! `RealRuntimeDirs` instead (`git_diff_plugin.rs`'s reason for the same
//! choice, see its module doc).
//!
//! Every expected word set below is written
//! by hand from the fixture's own text, never produced by the plugin's own
//! scan.

use super::*;

use std::path::Path;
use std::time::{Duration, Instant};

use hume_scripting::ScriptingHost;

use crate::editor::tests::lsp_rig::{LspRig, RUST_ANALYZER, RigSpec};

const BUFFER_WORDS_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/buffer-words/plugin.scm"
));

/// Stages the real `core:buffer-words` (and its `core:stdlib` dependency)
/// into `guard`'s isolated runtime and loads them. No buffer is open yet:
/// every test opens its own fixture afterward, via [`open`], so
/// `on-buffer-open` fires with the plugin's hook already registered
/// (`Editor::open`'s own startup buffer opens *before* any plugin loads).
fn setup(guard: &RuntimeDirs, tmp: &Path, config_expr: Option<&str>) -> Editor {
    write_core_plugin(guard, "buffer-words", BUFFER_WORDS_PLUGIN);
    write_core_plugin(guard, "stdlib", STDLIB_PLUGIN);
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {}), guard.dirs()).unwrap();
    let mut host = ScriptingHost::new(&ed.state.dirs);
    let load_bw = match config_expr {
        Some(cfg) => format!("(load-plugin! \"core:buffer-words\" #:config {cfg})"),
        None => "(load-plugin! \"core:buffer-words\")".to_string(),
    };
    let source = format!("(load-plugin! \"core:stdlib\")\n{load_bw}");
    eval_with_real_host(&mut ed, &mut host, &source, tmp);
    ed.scripting = Some(host);
    ed
}

/// Opens `path` as a real, file-backed buffer and returns its id.
fn open(ed: &mut Editor, path: &Path) -> BufferId {
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    ed.focused_buffer_id()
}

/// `src/main.rs` holding `marked`'s text, opened with `core:stdlib`,
/// `core:buffer-words` and `core:lsp` loaded and one `rust-analyzer`
/// attached over `backend`: handshake done, `on-lsp-attach` processed. The
/// caller holds a `RealRuntimeDirs`.
fn open_with_lsp(
    guard: &RealRuntimeDirs,
    tmp: &Path,
    marked: &str,
    backend: hume_lsp::test_util::RecordingLspBackend,
) -> Editor {
    let init = format!(
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:buffer-words\")\n(load-plugin! \"core:lsp\") (%activate-plugin-inline! \"core:lsp\" #f)\n{RUST_ANALYZER}"
    );
    let spec = RigSpec::rust(marked)
        .with_init(&init)
        .with_dirs(guard.dirs());
    let mut rig = LspRig::drained(tmp, spec, backend);
    rig.ed.settle();
    rig.ed
}

/// The ranked labels the open completion session would show, top 20: a
/// local copy of `tests/completion/mod.rs`'s `labels`, since that one is
/// private to a sibling module tree.
fn labels(ed: &Editor) -> Vec<String> {
    ed.state
        .input
        .buffer_completion()
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

/// Types `text` char by char without entering or leaving Insert mode,
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

/// Jumps to 1-based `line` and makes a one-character edit there (typed then
/// left in place), leaving the editor in Insert mode. `on-text-changed`
/// hands the hook only a buffer id, so this is how a test anchors the
/// background walk's next restart at a specific line: `:goto` alone moves
/// the cursor but fires no reindex on its own; only an actual edit does.
fn goto_line_and_edit(ed: &mut Editor, line: usize, ch: char) {
    type_cmd(ed, &format!(":{line}"));
    ed.feed_key(key('i'));
    ed.feed_key(key(ch));
    ed.feed_key(key_esc());
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
    let guard = RuntimeDirs::new();
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
/// tick's `"lines"` budget, is still found: the actual case a background
/// walk exists for. A whole-buffer read would also find this word, so the
/// test mainly checks `bw/walk!`'s per-tick fetch arithmetic: an off-by-one
/// in `fwd-hi`/`bwd-lo` would skip the line this fixture puts the target
/// word on, rather than merely being masked by overlap with an
/// already-covered line.
#[test]
fn a_word_many_lines_past_the_cursor_is_offered() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
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
    let guard = RuntimeDirs::new();
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
    // An empty menu would vacuously satisfy the assertion above without
    // proving anything. "helloworld"
    // also starts with the live prefix "hello" (so Rust's own re-ranking
    // doesn't filter it out the way it would something unrelated), but
    // isn't *equal* to it, pinning that the exclusion is specifically
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
    let guard = RuntimeDirs::new();
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

    // Backspace to "ca", not all the way to "cat": "cat" is itself a whole
    // word in the buffer, and the exact-token exclusion now correctly tracks
    // the live token as it's re-invoked (see
    // `the_exact_token_exclusion_tracks_further_typing`), so a live token
    // that exactly equals "cat" would exclude it, and accepting it right then
    // really would be a no-op. "ca" isn't a whole word, so both survive.
    ed.feed_key(key_backspace());
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
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    // A trailing space keeps "newword" out of the live token, so it isn't
    // excluded as the in-progress prefix (see `the_partially_typed_token_…`
    // above). This test is about the refresh, not the exclusion.
    let typed = "newword ";
    type_in_insert(&mut ed, typed);
    wait_for_word(&mut ed, "newword");

    for _ in 0..typed.chars().count() {
        ed.feed_key(key_backspace());
    }
    wait_for_word_gone(&mut ed, "newword");
}

/// A background walk finishing while the menu is already open must not
/// reset the user's navigation *twice* under one still-live invocation:
/// `bw/push-finished-answer!` clears `"live-id"` once used, so a *second*
/// walk completing with no fresh trigger in between is a no-op instead of
/// re-emitting under the same id and resetting the selection again. The
/// global `word-chars` option change is the trigger: `'on-option-change`
/// reindexes every open buffer without touching this buffer's own text or
/// cursor, so it can't also disturb the session's tokens. The one thing
/// under test here is `bw/push-finished-answer!` itself.
#[test]
fn a_second_background_finish_does_not_reset_the_menu_selection() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "alpha beta gamma\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key_down());
    ed.feed_key(key('i'));
    wait_for_word(&mut ed, "gamma");
    assert!(
        labels(&ed).len() >= 2,
        "sanity: enough candidates to move off row 0: {:?}",
        labels(&ed)
    );

    // A fresh reindex now legitimately re-emits under the trigger's still-
    // live id and resets the selection once: expected, not the bug under
    // test. `reindex_and_wait` drives one full cycle.
    reindex_via_option_change(&mut ed);
    assert_eq!(
        ed.state.input.completion_selected(),
        Some(0),
        "sanity: the first finish under a fresh id does reset the selection"
    );

    ed.feed_key(key_down());
    let moved = ed.state.input.completion_selected();
    assert_ne!(moved, Some(0), "sanity: selection moved off row 0");

    // A *second* reindex, no fresh trigger in between: `"live-id"` was
    // already consumed and cleared by the push above, so this one must be
    // a silent no-op.
    reindex_via_option_change(&mut ed);
    assert_eq!(
        ed.state.input.completion_selected(),
        moved,
        "a second background reindex finishing under the same already-\
         consumed id must not silently reset the user's navigation"
    );
}

/// Fires `'on-option-change` for `"word-chars"` (reindexing every open
/// buffer without touching this buffer's own text or cursor; see this
/// file's own test doc for why that matters) and waits, bounded, for the
/// walk it restarts to finish.
fn reindex_via_option_change(ed: &mut Editor) {
    ed.state.settings.word_chars = "-".into();
    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnOptionChange {
            key: "word-chars".into(),
            value: hume_scripting::host::OptionValue::Str("-".into()),
        });
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        ed.settle();
        if ed
            .state
            .input
            .buffer_completion()
            .is_none_or(|s| s.is_pending())
        {
            std::thread::sleep(Duration::from_millis(10));
            assert!(Instant::now() < deadline, "the walk never finished");
            continue;
        }
        break;
    }
}

/// A walk cancelled mid-way through a large buffer must not leave its
/// partial `"building"` set to survive into the fresh walk that replaces
/// it: `bw/reindex!` clears `"building"` in the same install that bumps
/// `"gen"`, so a word removed by the edit that triggered the reindex can't
/// resurface from a stale partial scan the old (cancelled) walk already
/// folded it into.
#[test]
fn a_reindex_that_interrupts_a_running_walk_does_not_resurrect_a_removed_word() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    // A small per-tick budget over many lines: the first tick (anchored at
    // line 0, where "targetword" lives) picks the word up almost
    // immediately, while the walk as a whole takes many further ticks,
    // wide enough a window to land the edit below before it finishes.
    let mut ed = setup(&guard, tmp.path(), Some(r#"(hash "lines" 5)"#));
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    let mut content = "targetword\n".to_string();
    content.push_str(&"filler\n".repeat(400));
    std::fs::write(&path, &content).unwrap();
    open(&mut ed, &path);

    // Give the `on-buffer-open`-triggered walk a few ticks to fold
    // "targetword" into "building" before interrupting it.
    ed.settle();
    std::thread::sleep(Duration::from_millis(80));
    ed.settle();

    // Delete "targetword" from line 1: the edit `on-text-changed`'s
    // debounced `bw/reindex!` restarts the walk over.
    type_cmd(&mut ed, ":1");
    ed.feed_key(key('d'));
    ed.feed_key(key('w'));
    ed.settle();

    ed.feed_key(key('i'));
    wait_for_word_gone(&mut ed, "targetword");
}

#[test]
fn fuzzy_match_is_opt_in() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "roundtrip\n\n").unwrap();

    let guard = RuntimeDirs::new();
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
    let guard2 = RuntimeDirs::new();
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
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    // Set the *global* default before opening the fixture, rather than a
    // buffer-scoped override after: the buffer this plugin cares about
    // doesn't exist yet, and setting it first means the very first index
    // (built synchronously by `on-buffer-open`) already sees it, with no
    // need to force a second reindex.
    type_cmd(&mut ed, ":set global word-chars=-");
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "foo-bar\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    trigger(&mut ed);
    let got = labels(&ed);
    assert!(got.contains(&"foo-bar".to_string()), "{got:?}");
    // Negative control: without this, "foo-bar" being present would just as
    // well be explained by "foo" and "bar" each separately matching the
    // (empty) prefix and Rust's own re-ranking coincidentally listing
    // "foo-bar" too, so it wouldn't prove the tokenizer actually joined them
    // into one word. Since "foo-bar" is the *only* text in the buffer, "foo"
    // and "bar" can only appear as their own labels if the scan split on
    // `-` instead of treating it as a word char.
    assert!(!got.contains(&"foo".to_string()), "{got:?}");
    assert!(!got.contains(&"bar".to_string()), "{got:?}");
}

/// Non-ASCII *punctuation* is a word boundary. An em dash between two words
/// must not merge them into one unreachable candidate. `split-words`
/// classifies exactly, so both halves are offered separately.
#[test]
fn non_ascii_punctuation_does_not_merge_the_words_around_it() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "foo\u{2014}bar\n").unwrap(); // em dash
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    trigger(&mut ed);
    let got = labels(&ed);
    assert!(got.contains(&"foo".to_string()), "{got:?}");
    assert!(got.contains(&"bar".to_string()), "{got:?}");
    assert!(!got.contains(&"foo\u{2014}bar".to_string()), "{got:?}");
}

/// Same rule for a different character: a curly apostrophe (U+2019) must split the
/// word around it rather than being absorbed into it.
#[test]
fn a_curly_apostrophe_splits_the_word_around_it() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "l\u{2019}\u{e9}l\u{e9}ment\n").unwrap(); // l'élément
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    trigger(&mut ed);
    let got = labels(&ed);
    assert!(got.contains(&"\u{e9}l\u{e9}ment".to_string()), "{got:?}");
}

/// A combining-mark accent must stay attached
/// to its word, not read as a boundary: `café` spelled as `e` + U+0301
/// (combining acute), not the precomposed codepoint, is the case that
/// actually exercises grapheme-cluster handling rather than a single-char
/// classification.
#[test]
fn a_combining_mark_stays_attached_to_its_word() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "cafe\u{0301} latte\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    trigger(&mut ed);
    let got = labels(&ed);
    assert!(got.contains(&"cafe\u{0301}".to_string()), "{got:?}");
    assert!(got.contains(&"latte".to_string()), "{got:?}");
}

#[test]
fn an_invalid_match_config_fails_the_load() {
    use hume_scripting::PluginStatus;
    use hume_scripting::attribution::PluginId;

    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    write_core_plugin(&guard, "buffer-words", BUFFER_WORDS_PLUGIN);
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);

    let init_path = tmp.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:buffer-words\" #:config (hash \"match\" 'regex))",
    )
    .unwrap();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {}), guard.dirs()).unwrap();
    let mut host = ScriptingHost::new(&ed.state.dirs);
    let effects = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("a failed plugin load must be contained, not abort eval_init");

    // eval_init queues log messages on the host (ctx.log) rather than
    // writing ed.state.message_log directly. Only Editor::init_scripting's
    // tail code flushes that queue, which this test bypasses by calling
    // eval_init directly, so check the host's queue itself.
    assert!(
        host.peek_pending_messages().iter().any(|(level, msg)| {
            matches!(level, hume_scripting::LogLevel::Error)
                && msg.contains("core:buffer-words")
                && msg.contains("must be one of")
        }),
        "error must carry the plugin's own prefixed message; got {:?}",
        host.peek_pending_messages()
    );

    ed.apply_script_effects(effects);
    ed.scripting = Some(host);

    let id = PluginId::Core("buffer-words".to_string());
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Failed)
        ),
        "core:buffer-words must be marked Failed after its body raises"
    );
}

/// Closing a buffer mid-walk must cancel its timer chain rather than leave
/// an orphaned continuation running against a since-replaced (or absent)
/// entry. There's no clean seam to assert the entry is actually gone from
/// Steel state, so this observes the property that matters from Rust: the
/// orphaned continuation, if the cancel didn't happen, would eventually
/// fire and either panic or write into whatever the same buffer id's entry
/// becomes next. Draining well past when the walk would have finished, with
/// no panic, is the achievable proxy for "no leak."
#[test]
fn closing_a_buffer_drops_its_index() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
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

/// A word past the point where the forward scan exhausts the buffer must
/// still be found by the backward scan: the common case once the cursor
/// is near the end.
///
/// `a_word_many_lines_past_the_cursor_is_offered` doesn't exercise this: it
/// anchors at line 0, where the forward side never empties out. This test
/// anchors at the buffer's *last* line instead (the forward side exhausts
/// on tick 1) and places the target word inside one of the backward
/// ticks' own windows (`"lines" 10`), at a fixed, predictable offset.
#[test]
fn a_word_before_the_cursor_survives_the_forward_side_emptying_out() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), Some(r#"(hash "lines" 10)"#));
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    // 60 lines, 1-based. Line 57 sits inside the backward window
    // (47..57) a tick fetches once the walk has stepped back to anchor 59:
    // offset 5 into that 10-line window, past the 4-element survivor cap
    // the bug leaves behind.
    let mut lines: Vec<String> = (1..=60)
        .map(|n| {
            if n == 57 {
                "buried_word".to_string()
            } else {
                "filler".to_string()
            }
        })
        .collect();
    lines.push(String::new());
    std::fs::write(&path, lines.join("\n")).unwrap();
    open(&mut ed, &path);
    goto_line_and_edit(&mut ed, 60, 'z');
    ed.feed_key(key('i'));
    wait_for_word(&mut ed, "buried_word");
}

/// Closing the *last* open buffer frees its slot and opens a fresh scratch
/// buffer under a brand new `BufferId`: a genuine open, so `bw/reindex!`
/// picks it up via the plugin's own `on-buffer-open` handler like any other
/// freshly opened buffer (see README.md's "Cursor-outward, line-windowed
/// indexing").
#[test]
fn typing_in_the_replacement_scratch_after_closing_the_last_buffer_is_indexed() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "alpha\n").unwrap();
    let bid = open(&mut ed, &path);
    // `Editor::open` always seeds a startup scratch buffer alongside `bid`
    // (`Editor::open`'s own doc), so `bid` isn't the *only* open buffer yet.
    // Closing it now would just switch focus to that scratch, not hit
    // the last-buffer path. Switch to it and close it first, so `bid` really
    // is the last buffer standing when it's closed next.
    type_cmd(&mut ed, ":bprev");
    ed.execute_typed("bd", None).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "closing the startup scratch must fall back to the only other buffer"
    );

    ed.execute_typed("bd", None).unwrap();
    assert!(
        ed.state.buffers.try_get(bid).is_none(),
        "the closed buffer's own slot must be freed, not reused"
    );
    assert_ne!(
        ed.focused_buffer_id(),
        bid,
        "the fresh scratch buffer must have its own, different BufferId"
    );
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "resurrected_word ");
    wait_for_word(&mut ed, "resurrected_word");
}

/// Restarting the walk while an earlier walk is still mid-flight doesn't
/// lose coverage at either end: the restarted walk starts from an empty word
/// set and still covers the whole buffer, with the `"gen"` guard keeping the
/// cancelled walk's ticks from interfering (README.md's "Staleness and
/// cancellation").
///
/// This does not reproduce the orphaned-tick race that guard covers: a tick
/// already dequeued off the timer wheel, firing after a fresh `bw/reindex!`
/// has installed a new generation. That needs `drain_due_timers` to pop the
/// walk's pending tick and a fresh debounce timer in the same batch, in that
/// order, which a wall-clock-driven test can't force: the walk's tick is due
/// far sooner (16ms) than a fresh debounce (150ms), so it always drains first.
#[test]
fn restarting_the_walk_mid_flight_still_reaches_both_ends() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), Some(r#"(hash "lines" 5)"#));
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    let mut lines: Vec<String> = (1..=160)
        .map(|n| match n {
            10 => "far_back".to_string(),
            150 => "far_fwd".to_string(),
            _ => "filler".to_string(),
        })
        .collect();
    lines.push(String::new());
    std::fs::write(&path, lines.join("\n")).unwrap();
    open(&mut ed, &path);
    goto_line_and_edit(&mut ed, 80, 'z');
    // Lets the debounce fire, starting the walk outward from line 80. Each
    // tick covers 5 lines in both directions, so the walk is about 16 ticks,
    // and the words reach the candidate list only once all of them have run.
    // Ticks run only when the editor settles, so none run during the sleep.
    std::thread::sleep(Duration::from_millis(200));
    ed.settle();
    // A second edit restarts the walk while the first is still mid-flight:
    // its remaining ticks need at least 15 * 16ms = 240ms, and the
    // restart's debounce fires after 150ms.
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "y");
    ed.feed_key(key_esc());
    ed.feed_key(key('i'));
    wait_for_word(&mut ed, "far_back");
    wait_for_word(&mut ed, "far_fwd");
}

/// `bw/reindex!` clamps the backward window's upper bound against the
/// buffer's *live* line count on every tick, the same way the forward
/// window's is already clamped (see README.md's "Cursor-outward,
/// line-windowed indexing"). Without it, a walk started on a large buffer
/// whose backward anchor is still well above 0 raises once the buffer
/// shrinks out from under it (`:e!` onto a shorter file, a big delete, an
/// LSP `applyEdit`) instead of adapting: `buffer-lines` raises on an
/// out-of-bounds range rather than clamping it.
#[test]
fn the_index_recovers_when_the_buffer_shrinks_mid_walk() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), Some(r#"(hash "lines" 5)"#));
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    let big_content = "filler\n".repeat(500);
    std::fs::write(&path, &big_content).unwrap();
    open(&mut ed, &path);
    goto_line_and_edit(&mut ed, 250, 'z');
    // The walk starts (debounce) and makes real progress outward from line
    // 250 in both directions, but 500 lines at "lines" 5 is far from done.
    std::thread::sleep(Duration::from_millis(200));
    ed.settle();

    std::fs::write(&path, "tiny\n").unwrap();
    ed.execute_typed("e!", None).unwrap();

    for _ in 0..15 {
        ed.settle();
        if let Some(msg) = &ed.state.status_msg {
            assert!(
                !msg.contains("buffer-lines"),
                "the stale walk's backward window must clamp instead of raising: {msg}"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    ed.feed_key(key('i'));
    wait_for_word(&mut ed, "tiny");
}

/// The exact-token exclusion (README.md's "Matching") filters against the
/// *live* token, not the prefix at the moment `Ctrl-Space` was first
/// pressed. This needs no re-invocation: `CompletionSession::rerank_
/// open_session` re-ranks the session's already-answered items against the
/// live token after every edit regardless, and `CompletionItem::
/// is_noop_for` is what that re-rank consults.
#[test]
fn the_exact_token_exclusion_tracks_further_typing() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "cat catalog\n\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key_down());
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "ca");
    trigger(&mut ed);
    let got = labels(&ed);
    assert!(got.contains(&"cat".to_string()), "{got:?}");

    ed.feed_key(key('t'));
    ed.settle();
    let got = labels(&ed);
    assert!(
        !got.contains(&"cat".to_string()),
        "typing to an exact word match must exclude it once re-invoked: {got:?}"
    );
    assert!(got.contains(&"catalog".to_string()), "{got:?}");
}

/// A trigger fired while the background walk is still partial does not
/// freeze that partial answer: once the walk finishes, it pushes its own
/// completed word list straight to the still-open invocation
/// (`bw/push-finished-answer!`, README.md's "Pushing a finished index to
/// an open menu"), with no further keystroke and no fresh `Ctrl-Space` required.
#[test]
fn a_menu_opened_before_the_walk_finishes_catches_up_once_it_does() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), Some(r#"(hash "lines" 20)"#));
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    let mut content = "filler\n".repeat(300);
    content.push_str("late_word\n");
    std::fs::write(&path, &content).unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    // One trigger, up front, before the background walk can possibly have
    // reached `late_word` (300 lines at "lines" 20 needs multiple ticks).
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    assert!(!labels(&ed).contains(&"late_word".to_string()));

    // No further keystroke: the walk's own push is what has to surface
    // `late_word`, once it finishes on its own timer chain.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        ed.settle();
        if labels(&ed).contains(&"late_word".to_string()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the walk's finished-answer push never arrived"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A *global* `word-chars` change reindexes every open buffer
/// (`on-option-change`, README.md's "`word-chars` invalidation"), unlike
/// `word_chars_extends_what_counts_as_a_word` above, which sets the option
/// before the buffer is even opened (so the very first index already sees
/// it), this sets it on an *already-indexed* buffer with no intervening
/// edit, which only the new hook can catch.
#[test]
fn a_global_word_chars_change_reindexes_an_already_open_buffer() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "foo-bar\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(
        !labels(&ed).contains(&"foo-bar".to_string()),
        "sanity: default word-chars must not join foo-bar yet: {:?}",
        labels(&ed)
    );

    // Two Escapes, not one: the first closes the still-open completion
    // popup from `trigger` above, the second leaves Insert mode. A single
    // Escape here would leave the editor in Insert mode, and the ":set..."
    // below would be typed as literal buffer text instead of run as a
    // command.
    ed.feed_key(key_esc());
    ed.feed_key(key_esc());
    type_cmd(&mut ed, ":set global word-chars=-");
    ed.feed_key(key('i'));
    wait_for_word(&mut ed, "foo-bar");
}

/// The plan's reason for existing: `core:buffer-words` and `core:lsp`
/// sharing one buffer, exercising multi-source ranking (`#:priority`) with
/// two real, independently-motivated sources. Nothing else in the
/// codebase does this today. `RealRuntimeDirs` rather than
/// `RuntimeDirs`: `core:lsp` is multi-file (see `git_diff_plugin.rs`'s
/// module doc for the same reasoning), and it picks up this plugin's own
/// real, just-shipped `runtime/plugins/core/buffer-words/plugin.scm` too,
/// no separate staging needed.
#[test]
fn buffer_words_and_lsp_rank_together_lsp_first() {
    use hume_lsp::test_util::RecordingLspBackend;

    let tmp = safe_tempdir();
    let guard = RealRuntimeDirs::new();

    let (mut backend, _notifications, requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({ "capabilities": { "completionProvider": {} } }),
    );
    backend.respond_to(
        "textDocument/completion",
        serde_json::json!([{"label": "lsp_item"}]),
    );

    let mut ed = open_with_lsp(&guard, tmp.path(), "-[b]>uffer_word\n", backend);

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

/// `core:buffer-words` never passes `#:resolve #t`, so accepting one of its
/// items must send no `completionItem/resolve` request, even in a buffer
/// whose attached server advertises `resolveProvider`. Before the
/// `#:resolve` gate, `accept`'s only condition was "no additionalTextEdits
/// and the buffer has a server with resolveProvider", true of any item in
/// this buffer, buffer-words' own `{"label": …}` items included, so the
/// server would have been sent an item it never produced.
#[test]
fn accepting_a_buffer_words_item_never_sends_completion_item_resolve() {
    use hume_lsp::test_util::RecordingLspBackend;

    let tmp = safe_tempdir();
    let guard = RealRuntimeDirs::new();

    let (mut backend, _notifications, requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({
            "capabilities": {
                "completionProvider": {"resolveProvider": true}
            }
        }),
    );
    backend.respond_to("textDocument/completion", serde_json::json!([]));

    // Mixed-case identifier, not `bw/case-twin`-eligible (an inner capital
    // means its tail isn't already all-lowercase). This test's exact-list
    // assertion needs buffer-words to answer with exactly one item.
    let mut ed = open_with_lsp(&guard, tmp.path(), "-[b]>ufferWord\n", backend);

    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.drain_lsp();
    ed.settle();

    let got = labels(&ed);
    assert_eq!(
        got,
        vec!["bufferWord"],
        "core:lsp answered empty; only buffer-words' item is on offer: {got:?}"
    );
    ed.feed_key(key_enter());
    ed.settle();

    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        0,
        "a buffer-words item must never trigger resolve, regardless of the buffer's server capabilities"
    );
}

/// The same identifier from `core:lsp` and `core:buffer-words` must show
/// once, not twice: `core:lsp`'s own plain (no textEdit, no
/// additionalTextEdits) item is a duplicate of buffer-words' own, and
/// `core:lsp`'s higher `#:priority` decides which one survives.
#[test]
fn a_plain_item_lsp_and_buffer_words_both_answer_is_shown_once_as_the_higher_priority_ones() {
    use hume_lsp::test_util::RecordingLspBackend;

    let tmp = safe_tempdir();
    let guard = RealRuntimeDirs::new();

    let (mut backend, _notifications, _requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({ "capabilities": { "completionProvider": {} } }),
    );
    backend.respond_to(
        "textDocument/completion",
        serde_json::json!([{"label": "bufferWord", "insertText": "bufferWord"}]),
    );

    // Mixed-case identifier, not `bw/case-twin`-eligible: see the sibling
    // test above's identical comment; this test needs LSP's and
    // buffer-words' items to carry the exact same one label for the dedup
    // check below to mean anything.
    let mut ed = open_with_lsp(&guard, tmp.path(), "-[b]>ufferWord\n", backend);

    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.drain_lsp();
    ed.settle();

    let got = labels(&ed);
    assert_eq!(
        got,
        vec!["bufferWord"],
        "the same label from two sources must be deduplicated to one row: {got:?}"
    );

    let top = ed
        .state
        .input
        .buffer_completion()
        .expect("session open")
        .top(20, &ed.state.config.completion_sources);
    assert_eq!(
        top[0]["source"], "lsp",
        "core:lsp's #:priority 10 beats buffer-words' default 0, so its own \
         (surviving) item is the one shown: {top:?}"
    );
}

/// Once anything is typed, `core:lsp`'s `'fuzzy` match (a real score above
/// `0`) always outranks `core:buffer-words`' own `'string` match (tied at
/// `0`), not merely "can", the way an earlier draft of this doc described
/// it. This is policy (`MatchKind::String`'s own doc,
/// `hume-editor/src/editor/completion/session.rs`), not a gap: in an
/// LSP-attached buffer, LSP should win once the user narrows by typing.
/// `#:priority` only ever decides a *tied* score, which this typed-prefix
/// case never reaches. Different labels here (unlike the dedup test
/// above) so nothing gets hidden as a duplicate and this pins ranking
/// alone.
#[test]
fn once_something_is_typed_lsp_always_outranks_buffer_words() {
    use hume_lsp::test_util::RecordingLspBackend;

    let tmp = safe_tempdir();
    let guard = RealRuntimeDirs::new();

    let (mut backend, _notifications, _requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({ "capabilities": { "completionProvider": {} } }),
    );
    backend.respond_to(
        "textDocument/completion",
        serde_json::json!([{"label": "buffer_analog", "insertText": "buffer_analog"}]),
    );

    let mut ed = open_with_lsp(&guard, tmp.path(), "-[b]>uffer_word_target\n\n", backend);

    // Second (blank) line, so buffer-words' own index (built from the
    // whole buffer, "buffer_word_target" included) has a real candidate
    // to offer for the "buf" typed here, distinct from the typed text
    // itself (which `is_noop_for` would otherwise drop as an exact match).
    ed.feed_key(key_down());
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "buf");
    trigger(&mut ed);
    ed.drain_lsp();
    ed.settle();

    let got = labels(&ed);
    assert!(
        got.iter().any(|l| l == "buffer_word_target"),
        "buffer-words' own candidate missing: {got:?}"
    );
    assert!(
        got.iter().any(|l| l == "buffer_analog"),
        "core:lsp's candidate missing: {got:?}"
    );
    assert_eq!(
        got.iter().position(|l| l == "buffer_analog"),
        Some(0),
        "core:lsp's real fuzzy score must beat buffer-words' flat 0, regardless of priority: {got:?}"
    );
}

/// A capitalized word (Titlecase: first letter up, rest down) also indexes
/// as its all-lowercase twin, so typing it mid-sentence (never capitalized
/// there) still finds it. See `plugin.scm`'s `bw/case-twin`. Covers a
/// non-ASCII first letter too: `Élan`'s twin is `élan`, not merely `Apply`'s
/// ASCII `apply`.
#[test]
fn a_capitalized_word_is_offered_lowercase_after_a_lowercase_prefix() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "Apply the patch. Élan vital.\n\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key_down());
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "ap");
    wait_for_word(&mut ed, "apply");
    let got = labels(&ed);
    assert!(!got.contains(&"Apply".to_string()), "{got:?}");

    for _ in 0..2 {
        ed.feed_key(key_backspace());
    }
    type_in_insert(&mut ed, "él");
    wait_for_word(&mut ed, "élan");
    let got = labels(&ed);
    assert!(!got.contains(&"Élan".to_string()), "{got:?}");
}

/// The reverse of the above: an all-lowercase word's Titlecase twin is
/// offered after an uppercase-led prefix, and only the twin (not the
/// as-written lowercase form), since that wouldn't match the case-sensitive
/// prefix gate at all.
#[test]
fn a_lowercase_word_is_offered_capitalized_after_an_uppercase_prefix() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "we apply it\n\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key_down());
    ed.feed_key(key('i'));
    type_in_insert(&mut ed, "Ap");
    wait_for_word(&mut ed, "Apply");
    let got = labels(&ed);
    assert!(!got.contains(&"apply".to_string()), "{got:?}");
}

/// A word with an inner capital (`HashMap`, `iPhone`) or in ALL-CAPS
/// (`MAX_LEN`) gets no twin at all: `bw/case-twin` only flips a *plain*
/// word's first letter, never one whose tail already carries case
/// information that flipping the head would destroy.
#[test]
fn mixed_case_and_all_caps_words_get_no_twin() {
    let tmp = safe_tempdir();
    let guard = RuntimeDirs::new();
    let mut ed = setup(&guard, tmp.path(), None);
    let file_dir = safe_tempdir();
    let path = file_dir.path().join("f.txt");
    std::fs::write(&path, "HashMap MAX_LEN iPhone\n\n").unwrap();
    open(&mut ed, &path);
    ed.feed_key(key_down());
    ed.feed_key(key('i'));

    type_in_insert(&mut ed, "Hash");
    wait_for_word(&mut ed, "HashMap");
    let got = labels(&ed);
    assert!(!got.contains(&"hashMap".to_string()), "{got:?}");

    for _ in 0..4 {
        ed.feed_key(key_backspace());
    }
    type_in_insert(&mut ed, "MAX");
    wait_for_word(&mut ed, "MAX_LEN");
    let got = labels(&ed);
    assert!(!got.contains(&"mAX_LEN".to_string()), "{got:?}");

    for _ in 0..3 {
        ed.feed_key(key_backspace());
    }
    type_in_insert(&mut ed, "iPho");
    wait_for_word(&mut ed, "iPhone");
    let got = labels(&ed);
    assert!(!got.contains(&"IPhone".to_string()), "{got:?}");
}
