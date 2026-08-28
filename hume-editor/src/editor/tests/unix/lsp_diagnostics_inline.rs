// Diagnostics end-of-line summary (`set-eol-text!`, wired from
// `on-diagnostics-changed` in `diagnostics.scm`) and the gn/gp
// dismiss-on-any-key overlay (`show-popup! #:kind 'scrollable`). Gutter signs
// from the same hook are covered by `lsp_diagnostic_signs.rs`, sharing this
// file's `setup_diagnostics` fixture (hoisted to `tests/unix/mod.rs`).

use super::*;

fn run(ed: &mut Editor, cmd: &str) {
    type_cmd(ed, cmd);
    ed.settle();
}

// ── End-of-line inline summary ──────────────────────────────────────────────

#[test]
fn single_diagnostic_on_a_line_shows_a_bare_message() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // Severity 1 = error, per the LSP DiagnosticSeverity enum.
    let diag: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
    let (ed, _guard) = setup_diagnostics(
        "aa\nbb\ncc\ndd\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );
    let bid = ed.focused_buffer_id();

    let entries: Vec<_> = ed
        .state
        .config
        .decorations
        .eol_text_for_buffer(bid)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1.pos, 3, "line 1's line-start char offset is 3");
    assert_eq!(
        entries[0].1.text, " problem A",
        "a single diagnostic must not get a '[1]' count prefix, but keeps \
         the leading space that separates it from the line's code"
    );
    assert_eq!(
        ed.view.registry.name_of(entries[0].1.scope),
        "diagnostic.error"
    );
}

#[test]
fn two_diagnostics_on_the_same_line_show_count_and_leftmost_message() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // Both on line1 ("bb", chars 3..5): D1 at col0 (char3), D2 at col1
    // (char4) — diagnostics-for-buffer is start-ascending, so D1 (leftmost)
    // supplies the message.
    let d1: DiagFixture = ((1, 0), (1, 1), 2, "warn near start");
    let d2: DiagFixture = ((1, 1), (1, 2), 1, "error further right");
    let (ed, _guard) = setup_diagnostics(
        "aa\nbb\ncc\ndd\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[d1, d2],
    );
    let bid = ed.focused_buffer_id();

    let entries: Vec<_> = ed
        .state
        .config
        .decorations
        .eol_text_for_buffer(bid)
        .collect();
    assert_eq!(entries.len(), 1, "both diagnostics collapse into one entry");
    assert_eq!(entries[0].1.pos, 3, "line 1's line-start char offset is 3");
    assert_eq!(
        entries[0].1.text, " [2] warn near start",
        "count prefix plus the leftmost (D1) diagnostic's message, with the \
         leading separator space"
    );
}

#[test]
fn inline_color_follows_the_highest_severity_on_the_line_not_the_leftmost() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // D1 (leftmost, col0) is a warning; D2 (col1) is an error. The message
    // must still come from D1, but the color must reflect D2's higher
    // severity.
    let d1: DiagFixture = ((1, 0), (1, 1), 2, "warn near start");
    let d2: DiagFixture = ((1, 1), (1, 2), 1, "error further right");
    let (ed, _guard) = setup_diagnostics(
        "aa\nbb\ncc\ndd\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[d1, d2],
    );
    let bid = ed.focused_buffer_id();

    let entries: Vec<_> = ed
        .state
        .config
        .decorations
        .eol_text_for_buffer(bid)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        ed.view.registry.name_of(entries[0].1.scope),
        "diagnostic.error",
        "an error anywhere on the line must win the color, even when the \
         leftmost (message-supplying) diagnostic is only a warning"
    );
}

/// `diagnostics-for-buffer` (no `#:severity`) must default to
/// `lsp.diagnostics-severity-floor`, same as the underline/gutter-sign
/// bridges — a below-floor diagnostic must not appear in the EOL summary
/// either. And raising the floor at runtime must refresh already-rendered
/// summaries via the `on-option-change` hook, not just future ones.
#[test]
fn eol_summary_respects_the_severity_floor_and_updates_when_it_changes() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((1, 0), (1, 2), 2, "just a warning"); // severity 2 = warning
    let (mut ed, _guard) = setup_diagnostics(
        "aa\nbb\ncc\ndd\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );
    let bid = ed.focused_buffer_id();

    assert_eq!(
        ed.state.config.decorations.eol_text_for_buffer(bid).count(),
        1,
        "sanity: floor defaults to hint, so the warning shows"
    );

    run(&mut ed, ":set global lsp.diagnostics-severity-floor=error");
    assert_eq!(
        ed.state.config.decorations.eol_text_for_buffer(bid).count(),
        0,
        "raising the floor above the diagnostic's severity must both stop \
         it appearing in future summaries and refresh the one already \
         rendered"
    );
}

#[test]
fn diagnostics_on_different_lines_get_independent_entries() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag_a: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
    let diag_b: DiagFixture = ((3, 0), (3, 2), 2, "problem B");
    let (ed, _guard) = setup_diagnostics(
        "aa\nbb\ncc\ndd\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag_a, diag_b],
    );
    let bid = ed.focused_buffer_id();

    let mut entries: Vec<(usize, String, String)> = ed
        .state
        .config
        .decorations
        .eol_text_for_buffer(bid)
        .map(|(_, e)| {
            (
                e.pos,
                e.text.clone(),
                ed.view.registry.name_of(e.scope).to_string(),
            )
        })
        .collect();
    entries.sort_by_key(|(pos, _, _)| *pos);
    // Line-start char offsets on this fixture: line 1 -> 3, line 3 -> 9.
    assert_eq!(
        entries,
        vec![
            (3, " problem A".to_string(), "diagnostic.error".to_string()),
            (
                9,
                " problem B".to_string(),
                "diagnostic.warning".to_string()
            ),
        ]
    );
}

// ── gn/gp full-message overlay ───────────────────────────────────────────────

#[test]
fn goto_next_diagnostic_opens_a_dismiss_on_key_popup_with_the_full_message() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((1, 0), (1, 2), 1, "problem A\nsecond line of detail");
    let (mut ed, _guard) = setup_diagnostics(
        "aa\nbb\ncc\ndd\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );

    // The real `g n` keybinding, not `:goto-next-diagnostic` — invoking via
    // the command line round-trips Command -> Normal mode, firing
    // hover.scm's unconditional `on-mode-change` -> `close-popup!` and
    // wiping the popup this same command just set. `g n` stays in Normal
    // mode throughout, matching real usage.
    ed.feed_key(key('g'));
    ed.feed_key(key('n'));

    let popup = ed.state.config.popup.as_ref().expect("popup must be shown");
    assert_eq!(
        popup.text, "problem A\nsecond line of detail",
        "the overlay must show the FULL message, not just its first line \
         (unlike the inline summary and the :diagnostics drawer row)"
    );
    assert!(
        matches!(popup.kind, hume_scripting::host::PopupKind::Scrollable),
        "the gn/gp overlay must be a dismiss-on-any-key popup, same kind as hover"
    );
}

#[test]
fn the_next_key_after_gn_dismisses_the_popup_but_still_executes() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag_a: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
    let diag_b: DiagFixture = ((3, 0), (3, 2), 1, "problem B");
    let (mut ed, _guard) = setup_diagnostics(
        "aa\nbb\ncc\ndd\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag_a, diag_b],
    );

    ed.feed_key(key('g'));
    ed.feed_key(key('n'));
    assert!(
        ed.state.config.popup.is_some(),
        "popup must be open after gn"
    );
    let line_before = ed.current_selections().primary().head();

    ed.feed_key(key('j')); // an ordinary Normal-mode motion, not a special dismiss key

    assert!(
        ed.state.config.popup.is_none(),
        "any key press must dismiss the overlay"
    );
    assert_ne!(
        ed.current_selections().primary().head(),
        line_before,
        "the dismissing key must still perform its own action (passive \
         dismiss, not swallowed)"
    );
}

#[test]
fn diagnostics_drawer_selection_does_not_open_a_popup() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag_a: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
    let diag_b: DiagFixture = ((3, 0), (3, 2), 2, "problem B");
    let (mut ed, _guard) = setup_diagnostics(
        "aa\nbb\ncc\ndd\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag_a, diag_b],
    );

    run(&mut ed, ":diagnostics");
    assert!(
        ed.state.config.popup.is_none(),
        "opening the drawer itself must not show a popup"
    );

    ed.handle_key(key('j'));
    ed.handle_key(key_enter());
    ed.settle();

    assert!(
        ed.state.config.popup.is_none(),
        "selecting a row in the :diagnostics drawer must jump without \
         opening the gn/gp overlay — only gn/gp show it"
    );
}
