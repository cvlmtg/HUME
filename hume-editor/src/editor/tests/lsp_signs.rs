// Diagnostic gutter signs: the `update_sign_providers`
// write side that feeds `SharedSignSource` (diagnostics + plugin `set-signs!`)
// from the diagnostics store and the signs store, plus the sign column's
// auto-collapsing width.
//
// Every test here goes through `Editor::open(None, std::sync::Arc::new(|| {}))` (not `editor_from`'s bare
// `Pane::new`) — sign providers are only registered by `build_pane`, same
// reasoning as `lsp_render.rs`.

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use hume_engine::builtins::sign_column::Sign;
use hume_engine::pipeline::{PaneId, RenderContext};
use hume_lsp::backend::LspBackend;
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// `((start_line, start_char), (end_line, end_char), severity)`.
type DiagFixture = ((u32, u32), (u32, u32), i64);

fn publish_diagnostics_notification(
    uri: &str,
    ranges_and_severity: &[DiagFixture],
) -> hume_lsp::codec::Message {
    let diagnostics: Vec<serde_json::Value> = ranges_and_severity
        .iter()
        .map(|&((sl, sc), (el, ec), severity)| {
            serde_json::json!({
                "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
                "severity": severity,
                "message": "boom",
            })
        })
        .collect();
    hume_lsp::codec::Message::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::json!({"uri": uri, "diagnostics": diagnostics}),
    }
}

struct DiagCtx {
    _file_dir: tempfile::TempDir,
    ed: Editor,
    pid: PaneId,
}

fn setup_with_diagnostics(content: &str, diags: &[DiagFixture]) -> DiagCtx {
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, content).unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    if !diags.is_empty() {
        backend.push_from_server(sid, publish_diagnostics_notification(uri.as_str(), diags));
    }

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, std::path::PathBuf::from(".")));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    ed.drain_lsp();

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    DiagCtx {
        _file_dir: file_dir,
        ed,
        pid,
    }
}

fn diag_signs(ed: &Editor, pid: PaneId) -> rustc_hash::FxHashMap<usize, Vec<Sign>> {
    ed.state.panes.render[pid]
        .signs
        .diagnostics
        .read()
        .unwrap()
        .clone()
}

fn plugin_signs(ed: &Editor, pid: PaneId) -> rustc_hash::FxHashMap<usize, Vec<Sign>> {
    ed.state.panes.render[pid]
        .signs
        .plugin
        .read()
        .unwrap()
        .clone()
}

fn sign_column_width(ed: &Editor, pid: PaneId) -> u8 {
    ed.view.panes[pid]
        .providers
        .gutter_columns()
        .next()
        .expect("sign column registered first")
        .width(0)
}

// ── Diagnostic signs ──────────────────────────────────────────────────────────

#[test]
fn error_line_gets_a_sign_with_the_error_scope() {
    let c = setup_with_diagnostics("abcdefgh\n", &[((0, 2), (0, 5), 1)]);
    let error_scope =
        c.ed.view
            .registry
            .get("diagnostic.error")
            .expect("interned by the write side");

    let signs = diag_signs(&c.ed, c.pid);
    assert_eq!(signs.len(), 1);
    let sign = signs[&0].first().expect("one sign on the error line");
    assert_eq!(sign.text, "●");
    assert_eq!(sign.scope, error_scope);
    assert_eq!(
        sign.slot, 0,
        "diagnostics are this buffer's only sign channel — slot 0"
    );
}

#[test]
fn error_beats_warning_on_the_same_line() {
    let c = setup_with_diagnostics(
        "abcdefgh\n",
        &[((0, 0), (0, 1), 2), ((0, 4), (0, 5), 1)], // warning then error, same line
    );
    let error_scope = c.ed.view.registry.get("diagnostic.error").unwrap();

    let signs = diag_signs(&c.ed, c.pid);
    assert_eq!(signs.len(), 1, "one line, one merged sign");
    let sign = signs[&0].first().expect("one sign on the line");
    assert_eq!(
        sign.scope, error_scope,
        "error must win over warning on the same line regardless of publish order"
    );
}

#[test]
fn multiline_diagnostic_marks_every_line_it_touches() {
    // "abc\ndef\n" — a diagnostic covering char 2 ('c') through char 6 ('f'),
    // crossing the line-0/line-1 boundary.
    let c = setup_with_diagnostics("abc\ndef\n", &[((0, 2), (1, 3), 1)]);
    let signs = diag_signs(&c.ed, c.pid);
    assert_eq!(
        signs.len(),
        2,
        "both lines the diagnostic touches get a sign"
    );
    assert!(signs.contains_key(&0));
    assert!(signs.contains_key(&1));
}

#[test]
fn zero_diagnostics_produce_no_signs() {
    let c = setup_with_diagnostics("abcdefgh\n", &[]);
    assert!(diag_signs(&c.ed, c.pid).is_empty());
}

// ── Gutter width ──────────────────────────────────────────────────────────────

#[test]
fn gutter_width_stays_at_default_with_no_signs_under_always_mode() {
    let c = setup_with_diagnostics("abcdefgh\n", &[]);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        2,
        "default is `always` — column stays visible even with no signs"
    );
}

#[test]
fn gutter_width_collapses_under_auto_mode_with_no_signs() {
    let mut c = setup_with_diagnostics("abcdefgh\n", &[]);
    let bid = c.ed.focused_buffer_id();
    c.ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("auto".parse().unwrap());
    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        0,
        "auto mode with no signs — column collapses"
    );
}

#[test]
fn gutter_width_is_the_default_when_a_diagnostic_exists() {
    let c = setup_with_diagnostics("abcdefgh\n", &[((0, 0), (0, 1), 1)]);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        2,
        "a diagnostic exists — column shows at the default width"
    );
}

#[test]
fn gutter_width_always_2_is_3_cells_wide() {
    let mut c = setup_with_diagnostics("abcdefgh\n", &[]);
    let bid = c.ed.focused_buffer_id();
    c.ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("always:2".parse().unwrap());
    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        3,
        "always:2 = 2 sign slots + 1 padding = 3 cells"
    );
}

#[test]
fn gutter_width_auto_2_expands_when_signs_exist() {
    let mut c = setup_with_diagnostics("abcdefgh\n", &[((0, 0), (0, 1), 1)]);
    let bid = c.ed.focused_buffer_id();
    c.ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("auto:2".parse().unwrap());
    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        3,
        "auto:2 with signs = 2 sign slots + 1 padding = 3 cells"
    );
}

#[test]
fn always_2_pins_width_even_with_only_one_channel_present() {
    let mut c = setup_with_diagnostics("abcdefgh\n", &[((0, 0), (0, 1), 1)]);
    let bid = c.ed.focused_buffer_id();
    c.ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("always:2".parse().unwrap());
    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        3,
        "pinned width stays fixed even though only one channel (diagnostics) is live"
    );
}

// ── Plugin signs ──────────────────────────────────────────────────────────────

#[test]
fn plugin_sign_via_set_signs_appears_in_the_plugin_map() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "warn-scope" 7)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let signs = plugin_signs(&ed, pid);
    assert_eq!(signs.len(), 1);
    let sign = signs[&0].first().expect("one sign on the line");
    assert_eq!(sign.text, "!");
    assert_eq!(
        sign.slot, 0,
        "this plugin sign is the buffer's only sign channel — slot 0"
    );
    let warn_scope = ed.view.registry.get("warn-scope").unwrap();
    assert_eq!(sign.scope, warn_scope);

    assert_eq!(
        sign_column_width(&ed, pid),
        2,
        "a plugin sign alone must also expand the gutter"
    );
}

/// With no `signcolumn` override, `always` auto-sizes to the buffer's
/// live sign-priority ladder — two plugin sources at distinct priorities
/// on the same line both claim their own slot, ordered highest-priority
/// first, without the user having to pin `always:2` for it. This is the
/// behavior the wandering-`+`-column bug report asked for: a channel's
/// column position is a property of its priority, stable buffer-wide, not
/// a function of what else happens to share the line.
#[test]
fn default_signcolumn_auto_sizes_to_show_every_channel_present() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
             (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let signs = plugin_signs(&ed, pid);
    assert_eq!(signs.len(), 1, "one line, one merged entry across sources");
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "two distinct priorities on the ladder — both get their own slot, unpinned"
    );
    assert_eq!(line_signs[0].text, "+", "priority 9 (vcs) — slot 0");
    assert_eq!(line_signs[1].text, "!", "priority 3 (linter) — slot 1");
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "auto-sized to 2 slots + 1 padding"
    );
}

/// Pinning `always:1` caps the column at one slot regardless of how many
/// distinct priorities the ladder holds — the lower-priority channel is
/// hidden buffer-wide, not just squeezed off this one line.
#[test]
fn pinned_single_slot_keeps_only_the_higher_priority_sign() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("always:1".parse().unwrap());
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
             (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let signs = plugin_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        1,
        "always:1 pins exactly one slot — the other priority's slot doesn't fit"
    );
    assert_eq!(
        line_signs[0].text, "+",
        "priority 9 (vcs) beats priority 3 (linter) for the one slot"
    );
}

/// Priority *is* the slot: two plugin sources at the *same* priority resolve
/// to the *same* slot and contend for it, even with `always:2` pinned — the
/// second slot goes unclaimed rather than absorbing the loser, because
/// nothing else on this line asked for it. The tie itself still resolves
/// deterministically by source name (ascending), not by call order:
/// `signs_for_buffer` (`SourceStore::for_buffer`) yields entries ascending
/// by source name, and the plugin merge (`update_sign_providers`) keeps the
/// first entry per slot — so `"linter"` wins even though `"vcs"` is armed
/// first here.
#[test]
fn two_plugin_sources_at_equal_priority_contend_for_one_slot() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("always:2".parse().unwrap());
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 5)))
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 5)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let signs = plugin_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        1,
        "both signs share one priority — one slot, one winner, even at width 2"
    );
    assert_eq!(
        line_signs[0].text, "!",
        "equal priority — \"linter\" wins the tie by source name (alphabetically \
         first), even though \"vcs\" was armed first"
    );
    assert_eq!(
        line_signs[0].slot, 0,
        "priority 5 is the buffer's only channel — slot 0"
    );
}

/// With `signcolumn=always:2` pinned, both distinct-priority sources fit
/// their own slot regardless of the ladder's actual length — same outcome
/// as auto-size here (2 live priorities), but via the pinned path instead
/// of `SignColumnConfig::slots_for`'s ladder-length fallback.
#[test]
fn wider_signcolumn_keeps_multiple_signs_per_line() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("always:2".parse().unwrap());
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
             (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let signs = plugin_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "signcolumn=always:2 keeps both signs on the line"
    );
    assert_eq!(line_signs[0].text, "+", "priority 9 first");
    assert_eq!(line_signs[1].text, "!", "priority 3 second");
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "always:2 = 2 sign slots + 1 padding = 3 cells"
    );
}

/// Cross-map merge: diagnostics and plugin signs are written to two separate maps by
/// `update_sign_providers`, then merged by the engine's `SignColumn`, which
/// holds one `SharedSignSource` per map (see `build_pane`). Every test above
/// checks the two maps in isolation; this one drives the pane's actual
/// registered `SignColumn` to prove a diagnostic sign and a plugin sign on
/// the *same* line both survive into one render, in priority order.
#[test]
fn diagnostic_and_plugin_sign_share_a_line_and_both_survive_the_merge() {
    let tmp = safe_tempdir();
    let mut c = setup_with_diagnostics("abcdefgh\n", &[((0, 0), (0, 1), 1)]);

    let bid = c.ed.focused_buffer_id();
    c.ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("always:2".parse().unwrap());

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut c.ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "warn-scope" 20)))))"#,
        tmp.path(),
    );
    c.ed.scripting = Some(host);
    type_cmd(&mut c.ed, ":arm");

    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);

    let rope = c.ed.state.buffers.get(bid).text().rope().clone();
    let gutter_ctx = hume_engine::providers::GutterRowCtx {
        mode: hume_engine::types::EditorMode::Normal,
        primary_head_line: 0,
        rope: &rope,
    };
    let col = c.ed.view.panes[c.pid]
        .providers
        .gutter_columns()
        .next()
        .expect("sign column registered first");
    let cells = col.render_row_cells(
        hume_engine::types::RowKind::LineStart { line_idx: 0 },
        &gutter_ctx,
    );
    assert_eq!(
        cells.len(),
        2,
        "always:2 keeps 2 slots; both the diagnostic sign and the plugin sign must survive"
    );
    assert_eq!(
        cells[0].as_str(),
        "!",
        "plugin sign priority 20 outranks the diagnostic's fixed priority 10 — slot 0"
    );
    assert_eq!(
        cells[1].as_str(),
        "●",
        "diagnostic's fixed priority 10 resolves to slot 1"
    );
}

/// The sign-priority ladder is built from the whole buffer, not the current
/// viewport — a diagnostic scrolled out of view still reserves its
/// priority's slot, so a lower-priority plugin sign on a visible line
/// doesn't slide into slot 0 just because the diagnostic isn't sharing this
/// particular frame with it.
#[test]
fn ladder_is_buffer_wide_not_viewport_restricted() {
    let content: String = "line\n".repeat(60);
    let mut c = setup_with_diagnostics(&content, &[((50, 0), (50, 1), 1)]);

    let tmp = safe_tempdir();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut c.ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "git-diff" (current-buffer) (list (list 0 "+" "diff.plus.gutter" 0)))))"#,
        tmp.path(),
    );
    c.ed.scripting = Some(host);
    type_cmd(&mut c.ed, ":arm");

    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);

    assert!(
        diag_signs(&c.ed, c.pid).is_empty(),
        "the diagnostic on line 50 is scrolled out of the 25-row viewport"
    );
    let signs = plugin_signs(&c.ed, c.pid);
    let sign = signs[&0].first().expect("plugin sign on line 0");
    assert_eq!(
        sign.slot, 1,
        "the off-screen diagnostic (priority 10) still reserves slot 0 on the \
         buffer-wide ladder — the visible priority-0 git sign is pushed to slot 1"
    );
}

/// Regression: a stored diagnostic computed against the pre-reload text can
/// end up with offsets past the new (shorter) content — before the fix,
/// `update_sign_providers` fed that straight into `char_to_line`, which
/// panics on an out-of-bounds char index. `:e!` must clear diagnostics for
/// the reloaded buffer, so this never even reaches the panicking path.
#[test]
fn reload_to_shorter_text_clears_stale_diagnostics_and_does_not_panic() {
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "one two three four five six\n").unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, std::path::PathBuf::from(".")));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();
    // Starts at 0 (so it still overlaps the post-reload visible range and
    // isn't filtered out by `for_range` before ever reaching the
    // panic-prone `char_to_line` call) but ends at 27 — the original
    // (longer) content's length, far past the much shorter reloaded
    // content's 4 chars below.
    let params = serde_json::json!({
        "uri": uri.as_str(),
        "diagnostics": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 27}},
            "severity": 1,
            "message": "boom",
        }],
    });
    ed.ingest_publish_diagnostics(sid, serde_json::from_value(params).unwrap());
    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "seed diagnostic must land before the reload"
    );

    std::fs::write(&file, "one\n").unwrap();
    ed.execute_typed("e!", None).unwrap();

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (0, 0),
        "reload must clear diagnostics computed against the pre-reload text"
    );

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // must not panic

    let signs = diag_signs(&ed, pid);
    assert!(
        signs.is_empty(),
        "no stale sign should remain after reload clears diagnostics"
    );
}
