// core:git-diff — end-to-end plugin tests.
//
// Loads the real, multi-file `runtime/plugins/core/git-diff/plugin.scm`
// against the repo's actual `runtime/` tree (`RealRuntimeGuard`, the same
// approach `lsp_hover.rs`/`lsp_packaging.rs` use for `core:lsp`, another
// multi-file plugin) — a real `git` subprocess does the fetching, and
// assertions read the Rust-side decoration stores the plugin's setter calls
// land in, never the plugin's own Steel-internal state (there is no clean
// seam to reach that from Rust — see `plugin-architecture.md`'s module
// isolation rule).
//
// Independent oracle: every expected sign/line/span below is derived by
// hand from the committed-vs-buffer text each fixture sets up, never by
// calling `diff-buffer-lines`/`diff-words` in the test itself.

use super::*;

use std::path::Path;
use std::time::Duration;

use super::super::render_snapshot::render_to_styled_string;
use hume_scripting::ScriptingHost;
use ratatui::layout::Rect;

const SOURCE: &str = "git-diff";

/// Writes `content` to `<dir>/<name>` and commits it — the working tree is
/// left holding `content` (git commit never touches the tree), so a test
/// that wants a dirty (uncommitted) buffer writes over it again afterward.
fn commit_file(dir: &Path, name: &str, content: &str, msg: &str) {
    std::fs::write(dir.join(name), content).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-q", "-m", msg]);
}

/// Loads the real `core:git-diff` plugin eagerly against the repo's actual
/// `runtime/` tree. `guard` must outlive every assertion — its `HUME_RUNTIME`
/// env var is what makes `git-diff`'s `manifest.scm`/`*.scm` siblings
/// resolvable at all.
fn setup(tmp: &Path, config_expr: Option<&str>) -> (Editor, RealRuntimeGuard) {
    let guard = RealRuntimeGuard::new();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let mut host = ScriptingHost::new();
    let load = match config_expr {
        Some(cfg) => format!("(load-plugin \"core:git-diff\" #:config {cfg})"),
        None => "(load-plugin \"core:git-diff\")".to_string(),
    };
    eval_with_real_host(&mut ed, &mut host, &load, tmp);
    ed.scripting = Some(host);
    (ed, guard)
}

/// Opens `path` as a real, file-backed buffer (mirrors `lsp_hover.rs`'s
/// `setup`) and returns its id.
fn open(ed: &mut Editor, path: &Path) -> BufferId {
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    ed.focused_buffer_id()
}

/// Waits out a just-queued hook's 150ms `debounce-by` timer plus the
/// subsequent async `git show` round trip — for a bounded negative wait
/// (asserting nothing/something-specific happened) where no store mutation
/// exists to `drain_until` on.
///
/// The leading `settle()` matters as much as the sleep: a hook queued by
/// `execute_typed`/`feed_key` (`on-buffer-open`, `on-text-changed`,
/// `on-buffer-save`, …) only *runs* — and so only *starts* its debounce
/// timer — once something drains `pending_work`. Sleeping first and
/// settling once after, with no settle before the sleep, lets the timer's
/// own 150ms elapse before it has even been scheduled, so the wait
/// accomplishes nothing and the assertion after it passes vacuously.
fn wait_for_refresh(ed: &mut Editor) {
    ed.settle();
    std::thread::sleep(Duration::from_millis(400));
    ed.settle();
}

/// `signs_for(SOURCE, bid)`, remapped from char-offset `pos` back to a line
/// number and sorted by line — the shape a fixture's hand-derived
/// expectation can be written against directly.
fn signs(ed: &Editor, bid: BufferId) -> Vec<(usize, String, String, i64)> {
    let text = ed.state.buffers.get(bid).text();
    let mut v: Vec<_> = ed
        .state
        .config
        .decorations
        .signs_for(SOURCE, bid)
        .iter()
        .map(|e| {
            (
                text.char_to_line(e.pos),
                e.text.clone(),
                e.scope.clone(),
                e.priority,
            )
        })
        .collect();
    v.sort_by_key(|(line, ..)| *line);
    v
}

/// `line_backgrounds_for(SOURCE, bid)`, remapped to line numbers and sorted.
fn line_bgs(ed: &Editor, bid: BufferId) -> Vec<(usize, String)> {
    let text = ed.state.buffers.get(bid).text();
    let mut v: Vec<_> = ed
        .state
        .config
        .decorations
        .line_backgrounds_for(SOURCE, bid)
        .iter()
        .map(|e| (text.char_to_line(e.pos), e.scope.clone()))
        .collect();
    v.sort_by_key(|(line, _)| *line);
    v
}

/// `virtual_lines_for(SOURCE, bid)`, remapped to `(line, before, text,
/// scope, segments)` and sorted.
fn vlines(
    ed: &Editor,
    bid: BufferId,
) -> Vec<(
    usize,
    bool,
    String,
    Option<String>,
    Vec<(usize, usize, String)>,
)> {
    let text = ed.state.buffers.get(bid).text();
    let mut v: Vec<_> = ed
        .state
        .config
        .decorations
        .virtual_lines_for(SOURCE, bid)
        .iter()
        .map(|e| {
            (
                text.char_to_line(e.pos),
                e.before,
                e.text.clone(),
                e.scope.clone(),
                e.segments.clone(),
            )
        })
        .collect();
    v.sort_by_key(|(line, ..)| *line);
    v
}

/// `extra_highlights_for(SOURCE, bid)`, paired with the live buffer
/// substring each span covers — lets a test assert "this span covers the
/// changed word" without predicting `diff-words`' exact tokenization.
fn highlights(ed: &Editor, bid: BufferId) -> Vec<(usize, usize, String, String)> {
    let text = ed.state.buffers.get(bid).text();
    ed.state
        .config
        .decorations
        .extra_highlights_for(SOURCE, bid)
        .iter()
        .map(|e| {
            (
                e.start,
                e.end,
                e.scope.clone(),
                text.slice(e.start..e.end).to_string(),
            )
        })
        .collect()
}

// ── Gutter signs ─────────────────────────────────────────────────────────────

#[test]
fn signs_pure_addition_marks_one_plus_per_line() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\ntwo\nthree\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "one\nALPHA\nBETA\ntwo\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), None);
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .signs_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        signs(&ed, bid),
        vec![
            (1, "+".to_string(), "diff.plus.gutter".to_string(), 0),
            (2, "+".to_string(), "diff.plus.gutter".to_string(), 0),
        ],
        "a 20-line paste should show one + per line, not one per hunk"
    );
}

#[test]
fn signs_change_marks_tilde_per_line() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "a\nb\nc\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "a\nCHANGED\nc\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), None);
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .signs_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        signs(&ed, bid),
        vec![(1, "~".to_string(), "diff.delta.gutter".to_string(), 0)]
    );
}

#[test]
fn signs_pure_deletion_marks_line_above_gap() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "a\nb\nc\nd\ne\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "a\nb\nd\ne\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), None);
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .signs_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        signs(&ed, bid),
        vec![(1, "-".to_string(), "diff.minus.gutter".to_string(), 0)],
        "deleting 'c' must mark line 1 ('b'), the line above the gap"
    );
}

#[test]
fn signs_deletion_at_start_clamps_to_line_zero() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "a\nb\nc\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "b\nc\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), None);
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .signs_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        signs(&ed, bid),
        vec![(0, "-".to_string(), "diff.minus.gutter".to_string(), 0)],
        "deleting the first line must clamp to line 0, never go negative"
    );
}

#[test]
fn signs_deletion_at_end_of_file_marks_last_content_line() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "a\nb\nc\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "a\nb\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), None);
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .signs_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        signs(&ed, bid),
        vec![(1, "-".to_string(), "diff.minus.gutter".to_string(), 0)],
        "deleting the last line ('c') must mark line 1 ('b') without an \
         out-of-range set-signs! call — new-start (2) equals the buffer's \
         content line count"
    );

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(errors.is_empty(), "no out-of-range error; got {errors:?}");
}

// ── Inline rendering ────────────────────────────────────────────────────────

#[test]
fn inline_change_renders_virtual_line_word_spans_and_tint() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\nfoo bar baz\nthree\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "one\nfoo QUX baz\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), Some(r#"(hash "inline" #t)"#));
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .virtual_lines_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        vlines(&ed, bid),
        vec![(
            0,
            false,
            "foo bar baz".to_string(),
            Some("diff.minus".to_string()),
            vec![(4, 7, "diff.minus.word".to_string())],
        )],
        "the removed ref line anchors after line 0, carries the whole old \
         text, and underlines just the removed word ('bar')"
    );
    assert_eq!(
        highlights(&ed, bid),
        vec![(8, 11, "diff.plus.word".to_string(), "QUX".to_string())],
        "the new-side span must cover the live buffer's replacement word"
    );
    assert_eq!(line_bgs(&ed, bid), vec![(1, "diff.delta".to_string())]);
    assert_eq!(
        signs(&ed, bid),
        vec![(1, "~".to_string(), "diff.delta.gutter".to_string(), 0)]
    );
}

#[test]
fn inline_tab_indented_deletion_keeps_a_literal_tab_that_still_renders_at_the_right_display_column()
{
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\n\ttabbed line\nthree\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "one\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), Some(r#"(hash "inline" #t)"#));
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .virtual_lines_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        vlines(&ed, bid),
        vec![(
            0,
            false,
            "\ttabbed line".to_string(),
            Some("diff.minus".to_string()),
            Vec::new(),
        )],
        "set-virtual-lines! now accepts a literal tab in 'text and no longer \
         expands it — the engine expands it at render time instead, the \
         same as a real buffer line's tab"
    );
    assert_eq!(
        line_bgs(&ed, bid),
        Vec::<(usize, String)>::new(),
        "a pure deletion has no live new-side line to tint"
    );

    // Frame-level: the tab must still land on the default 4-wide stop on
    // screen, exactly as it did when the plugin expanded it by hand.
    let snap = render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn inline_wide_cjk_before_tab_in_a_deletion_shifts_the_tab_on_screen() {
    // The bug this whole change fixes: the plugin used to expand a deleted
    // line's tabs itself, counting one Steel char (not one display column)
    // per preceding character — a wide CJK grapheme before a tab landed
    // that tab one column early. Now the plugin stores the line verbatim
    // and the engine expands it, so a wide char correctly shifts the stop
    // by its full 2-column width.
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\n\u{6F22}\ttabbed\nthree\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "one\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), Some(r#"(hash "inline" #t)"#));
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .virtual_lines_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        vlines(&ed, bid),
        vec![(
            0,
            false,
            "\u{6F22}\ttabbed".to_string(),
            Some("diff.minus".to_string()),
            Vec::new(),
        )],
        "the stored text is the line verbatim — no plugin-side expansion"
    );

    // 漢 occupies columns 0-1, so the tab (tab_width 4) advances from
    // column 2 to column 4 — not column 3, which a char-counting (not
    // column-counting) expansion would have produced.
    let snap = render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn inline_pure_addition_has_no_virtual_line_only_tint() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\nthree\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "one\ntwo\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), Some(r#"(hash "inline" #t)"#));
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .line_backgrounds_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        vlines(&ed, bid),
        Vec::new(),
        "nothing was removed, so a pure addition contributes no virtual row"
    );
    assert_eq!(line_bgs(&ed, bid), vec![(1, "diff.plus".to_string())]);
}

// ── Commands and shared state ────────────────────────────────────────────────

#[test]
fn bare_toggle_off_clears_only_that_rendering() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\nfoo\nthree\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "one\nbar\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), Some(r#"(hash "signs" #t "inline" #t)"#));
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .signs_for(SOURCE, bid)
            .is_empty()
    });
    assert!(!signs(&ed, bid).is_empty(), "sanity: signs on");
    assert!(!line_bgs(&ed, bid).is_empty(), "sanity: inline on");

    type_cmd(&mut ed, ":toggle-git-signs");
    ed.settle();

    assert!(
        signs(&ed, bid).is_empty(),
        "a bare toggle off must clear the toggled rendering"
    );
    assert!(
        !line_bgs(&ed, bid).is_empty(),
        "the other, still-enabled rendering must be untouched"
    );
}

#[test]
fn explicit_ref_toggle_sets_ref_and_re_renders_the_other_enabled_rendering() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\ntwo\nthree\n", "v1");
    commit_file(repo.path(), "f.txt", "one\nCHANGED\nthree\n", "v2");
    // Working tree now matches HEAD (v2) exactly — the default-ref diff is
    // empty, isolating the effect of the explicit-ref toggle below.

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), Some(r#"(hash "signs" #t "inline" #f)"#));
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    // Let the buffer-open-triggered default-ref (HEAD) refresh finish first
    // — otherwise its debounced fetch, captured with the pre-toggle ref,
    // can complete after the explicit-ref toggle below and overwrite its
    // freshly fetched HEAD~1 blob with HEAD's (empty-diff) one.
    wait_for_refresh(&mut ed);

    // Turn inline on, explicitly pointed at the older commit.
    type_cmd(&mut ed, ":toggle-inline-diff HEAD~1");
    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .virtual_lines_for(SOURCE, bid)
            .is_empty()
    });

    assert_eq!(
        vlines(&ed, bid)
            .into_iter()
            .map(|(line, before, text, ..)| (line, before, text))
            .collect::<Vec<_>>(),
        vec![(0, false, "two".to_string())],
        "must diff against HEAD~1 ('two'), not the config default HEAD"
    );
    assert_eq!(line_bgs(&ed, bid), vec![(1, "diff.delta".to_string())]);
    assert_eq!(
        signs(&ed, bid),
        vec![(1, "~".to_string(), "diff.delta.gutter".to_string(), 0)],
        "signs stayed enabled the whole time — setting the ref from the \
         inline command must re-render it too, not just inline"
    );

    // Sticky across a bare off/on cycle: still HEAD~1, not back to HEAD.
    type_cmd(&mut ed, ":toggle-inline-diff");
    ed.settle();
    assert!(vlines(&ed, bid).is_empty(), "sanity: bare toggle off");

    type_cmd(&mut ed, ":toggle-inline-diff");
    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .virtual_lines_for(SOURCE, bid)
            .is_empty()
    });
    assert_eq!(
        vlines(&ed, bid)
            .into_iter()
            .map(|(line, before, text, ..)| (line, before, text))
            .collect::<Vec<_>>(),
        vec![(0, false, "two".to_string())],
        "a bare re-toggle must keep the last explicit ref (HEAD~1), not \
         reset to the config default — HEAD would show no diff at all here"
    );
}

// ── Lifecycle and failure ────────────────────────────────────────────────────

#[test]
fn untracked_file_shows_no_diff_and_logs_nothing() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "README", "readme\n", "init");
    std::fs::write(repo.path().join("new.txt"), "hello\nworld\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), None);
    let bid = open(&mut ed, &repo.path().join("new.txt"));
    // Opening any file itself reports an Info status ("Opened new.txt"),
    // which also lands in status_msg — capture it so the assertion below
    // isolates the plugin's own (silent) behavior from that unrelated message.
    let status_after_open = ed.state.status_msg.clone();

    // No positive signal exists for "the fetch ran and found nothing" —
    // wait past the 150ms debounce plus a real `git show` round trip,
    // mirroring `lsp_sighelp.rs`'s bounded-sleep idiom for a background
    // subprocess with no observable completion event to poll on.
    wait_for_refresh(&mut ed);

    assert!(
        signs(&ed, bid).is_empty(),
        "an untracked file must show no diff"
    );
    assert_eq!(
        ed.state.status_msg, status_after_open,
        "an untracked file's failed fetch is the silent 'trace branch — it \
         must not overwrite the status line with a warning/error of its own"
    );
}

#[test]
fn explicit_bad_ref_logs_warning_status_message() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\ntwo\nthree\n", "v1");

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), None);
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    type_cmd(&mut ed, ":toggle-git-signs not-a-real-ref-xyz");
    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    let msg = ed.state.status_msg.clone().unwrap();
    assert!(
        msg.contains("git-diff"),
        "a bad ref given explicitly must surface a status message; got {msg:?}"
    );
    assert!(
        signs(&ed, bid).is_empty(),
        "a failed explicit-ref fetch must not leave stale signs painted"
    );
}

#[test]
fn buffer_save_invalidates_cached_ref_and_refetches() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\ntwo\nthree\n", "v1");
    commit_file(repo.path(), "f.txt", "one\nCHANGED\nthree\n", "v2");
    // Dirty the working tree back to v1 — buffer opens differing from HEAD (v2).
    std::fs::write(repo.path().join("f.txt"), "one\ntwo\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), Some(r#"(hash "inline" #t)"#));
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .virtual_lines_for(SOURCE, bid)
            .is_empty()
    });
    assert_eq!(
        vlines(&ed, bid)[0].2,
        "CHANGED",
        "sanity: initial fetch cached HEAD (v2)'s blob"
    );

    // HEAD moves while the buffer stays open (an external commit) — the
    // plugin has no way to know, so its cached ref-text blob goes stale.
    commit_file(repo.path(), "f.txt", "one\nFINAL\nthree\n", "v3");

    // Insert then undo — a small, in-place edit (unlike a full `:e!`
    // reload, which replaces the whole buffer and would disturb every
    // decoration's remapped anchor regardless of the plugin's own logic) —
    // fires on-text-changed with the buffer's content unchanged, isolating
    // its effect from on-buffer-save's.
    ed.feed_key(key('i'));
    ed.feed_key(key('!'));
    ed.feed_key(key_esc());
    ed.feed_key(key('u'));
    wait_for_refresh(&mut ed);
    assert_eq!(
        vlines(&ed, bid)[0].2,
        "CHANGED",
        "on-text-changed alone must not invalidate the cached ref blob — \
         still diffing against the stale v2 cache, not the real v3 HEAD"
    );

    // Forced: the external v3 commit above changed f.txt's on-disk mtime
    // under this buffer, and a plain `:w` would otherwise refuse with
    // "file has changed on disk" — a real, separate protection this test
    // isn't exercising.
    ed.execute_typed("w!", None).unwrap();
    ed.settle(); // runs on-buffer-save, clearing the cached ref-text
    // Unlike the local-diff check above, this refetch needs a real
    // subprocess round trip (ref-text is now #f) — `drain_until`, not a
    // fixed sleep+settle, since the timer firing and the process completing
    // are two separate async stages that don't land in the same settle().
    drain_until(&mut ed, |ed| {
        vlines(ed, bid).first().map(|v| v.2.as_str()) == Some("FINAL")
    });
    assert_eq!(
        vlines(&ed, bid)[0].2,
        "FINAL",
        "on-buffer-save must invalidate the cached ref blob and refetch \
         the real, current HEAD (v3)"
    );
}

#[test]
fn buffer_close_after_open_leaves_no_stray_error() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\ntwo\nthree\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "one\nCHANGED\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), None);
    open(&mut ed, &repo.path().join("f.txt"));

    // Let on-buffer-open run (starting its debounce timer), then close just
    // past the 150ms debounce — the background `git show` is plausibly still
    // in flight at that point. on-buffer-close's cancel-fetch!/remove-buffer!
    // must leave no callback able to misfire against this now-closed buffer.
    ed.settle();
    std::thread::sleep(Duration::from_millis(160));
    ed.execute_typed("bd", None).unwrap();
    wait_for_refresh(&mut ed);

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "a stray post-close callback must not error; got {errors:?}"
    );
}

// ── Config ────────────────────────────────────────────────────────────────────

#[test]
fn config_flips_default_signs_and_inline() {
    let repo = safe_tempdir();
    git_init(repo.path());
    commit_file(repo.path(), "f.txt", "one\ntwo\nthree\n", "v1");
    std::fs::write(repo.path().join("f.txt"), "one\nCHANGED\nthree\n").unwrap();

    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup(tmp.path(), Some(r#"(hash "signs" #f "inline" #t)"#));
    let bid = open(&mut ed, &repo.path().join("f.txt"));

    drain_until(&mut ed, |ed| {
        !ed.state
            .config
            .decorations
            .line_backgrounds_for(SOURCE, bid)
            .is_empty()
    });

    assert!(
        signs(&ed, bid).is_empty(),
        "signs must start off when #:config sets \"signs\" #f"
    );
    assert_eq!(
        line_bgs(&ed, bid),
        vec![(1, "diff.delta".to_string())],
        "inline must start on when #:config sets \"inline\" #t"
    );
}

#[test]
fn bad_config_value_fails_plugin_load_with_prefixed_error() {
    use crate::editor::scripting_setup::make_init_host;
    use hume_scripting::PluginStatus;
    use hume_scripting::attribution::PluginId;

    let tmp = safe_tempdir();
    let _guard = RealRuntimeGuard::new();
    let init_path = tmp.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(load-plugin "core:git-diff" #:config (hash "signs" "yes"))"#,
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let err = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect_err("a non-boolean \"signs\" value must fail eval_init");

    assert!(
        err.message.contains("core:git-diff") && err.message.contains("must be #t or #f"),
        "error must carry the plugin's own prefixed message; got {:?}",
        err.message
    );

    ed.apply_script_effects(err.effects);
    ed.scripting = Some(host);

    let id = PluginId::Core("git-diff".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "core:git-diff must be marked Failed after its body raises"
    );
}
