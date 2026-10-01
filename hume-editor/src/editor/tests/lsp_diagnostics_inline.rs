// EOL text: the render-provider half that runs on every platform. The
// Steel-driven summary/overlay tests (which load the real `core:lsp` plugin)
// live in `unix/lsp_diagnostics_inline.rs`.

use super::*;
use hume_decorations::EolTextEntry;
use hume_engine::pipeline::RenderContext;

/// `update_eol_text_providers` (`decoration_providers.rs`) must hand the full,
/// untruncated message through to the pane's `InlineInsert`: the per-line
/// summary text set via `set-eol-text!` must reach the render provider
/// byte-for-byte. (`format_buffer_line`'s trailing-insert path then splits
/// this `InlineInsert` into one cell per grapheme so a terminal flush
/// doesn't clobber it past the first column, covered directly by
/// `format::tests::trailing_insert_emits_one_cell_per_grapheme` in
/// `hume-engine`, since a rendered-grid snapshot here can't observe that
/// terminal-flush-time truncation.)
#[test]
fn full_message_reaches_the_render_provider_untruncated() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "let x = 1".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let bid = ed.focused_buffer_id();
    let message = " mismatched types here";
    let scope = ed.view.registry.intern("diagnostic.error");
    ed.state.config.decorations.set_eol_text(
        "lsp".to_string(),
        bid,
        vec![EolTextEntry {
            pos: co(0),
            text: message.to_string(),
            scope,
            hide_on_insert_line: false,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(60, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focus.id();
    let by_line = ed.state.panes.render.get(pid).unwrap().eol_text();
    let inserts = by_line
        .get(&hume_rope::line::ContentLine::new(0))
        .expect("line 0 must have an insert");
    assert_eq!(inserts.len(), 1);
    assert_eq!(
        inserts[0].text, message,
        "the full message must reach the provider, not a prefix of it"
    );
}

/// Two entries from the *same* source landing on the same line: the shape a
/// remap produces when an edit collapses several originally-distinct lines
/// into one; `last_writer_per_line` folds them, keeping the last. Pushing
/// onto a per-line `Vec` in `update_eol_text_providers` would keep both
/// entries and render them concatenated at the same byte offset.
#[test]
fn two_entries_from_one_source_on_the_same_line_collapse_to_the_last_one() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "let x = 1".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let bid = ed.focused_buffer_id();
    let error_scope = ed.view.registry.intern("diagnostic.error");
    let warning_scope = ed.view.registry.intern("diagnostic.warning");
    ed.state.config.decorations.set_eol_text(
        "diagnostics".to_string(),
        bid,
        vec![
            EolTextEntry {
                pos: co(0),
                text: "first".to_string(),
                scope: error_scope,
                hide_on_insert_line: false,
            },
            EolTextEntry {
                pos: co(0),
                text: "second".to_string(),
                scope: warning_scope,
                hide_on_insert_line: false,
            },
        ],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(60, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focus.id();
    let by_line = ed.state.panes.render.get(pid).unwrap().eol_text();
    let inserts = by_line
        .get(&hume_rope::line::ContentLine::new(0))
        .expect("line 0 must have an insert");
    assert_eq!(
        inserts.len(),
        1,
        "two same-line entries from one source must collapse to one insert, \
         not stack"
    );
    assert_eq!(
        inserts[0].text, "second",
        "the later entry must win: last_writer_per_line folds left-to-right"
    );
}

/// Two sources tinting the same line: the cross-source
/// tie-break, mirroring the sign pipeline: the alphabetically *first*
/// source wins.
#[test]
fn two_sources_on_the_same_line_break_ties_alphabetically_first() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "let x = 1".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let bid = ed.focused_buffer_id();
    let scope = ed.view.registry.intern("diagnostic.error");
    ed.state.config.decorations.set_eol_text(
        "z-plugin".to_string(),
        bid,
        vec![EolTextEntry {
            pos: co(0),
            text: "from-z".to_string(),
            scope,
            hide_on_insert_line: false,
        }],
    );
    ed.state.config.decorations.set_eol_text(
        "a-plugin".to_string(),
        bid,
        vec![EolTextEntry {
            pos: co(0),
            text: "from-a".to_string(),
            scope,
            hide_on_insert_line: false,
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(60, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focus.id();
    let by_line = ed.state.panes.render.get(pid).unwrap().eol_text();
    let inserts = by_line
        .get(&hume_rope::line::ContentLine::new(0))
        .expect("line 0 must have an insert");
    assert_eq!(inserts.len(), 1);
    assert_eq!(
        inserts[0].text, "from-a",
        "the alphabetically first source (\"a-plugin\") must win"
    );
}

/// Two-line buffer, eol entries on both lines from `source`; returns the
/// editor in Normal mode with the cursor on line 0.
fn two_line_editor_with_eol(
    entries: &[(&str, usize, &str, bool)],
) -> (Editor, hume_engine::pipeline::PaneId) {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "abc\ndef".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    ed.feed_key(key('g'));
    ed.feed_key(key('g'));
    let bid = ed.focused_buffer_id();
    let scope = ed.view.registry.intern("diagnostic.error");
    for &(source, pos, text, hide) in entries {
        ed.state.config.decorations.set_eol_text(
            source.to_string(),
            bid,
            vec![EolTextEntry {
                pos: co(pos),
                text: text.to_string(),
                scope,
                hide_on_insert_line: hide,
            }],
        );
    }
    let pid = ed.state.focus.id();
    (ed, pid)
}

fn eol_lines(ed: &mut Editor, pid: hume_engine::pipeline::PaneId) -> Vec<usize> {
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(60, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let mut lines: Vec<usize> = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .eol_text()
        .keys()
        .map(|l| l.index())
        .collect();
    lines.sort_unstable();
    lines
}

#[test]
fn flagged_eol_text_is_dropped_on_the_insert_cursor_line_only() {
    let (mut ed, pid) =
        two_line_editor_with_eol(&[("lsp", 0, "e0", true), ("lsp2", 4, "e1", true)]);
    assert_eq!(
        eol_lines(&mut ed, pid),
        vec![0, 1],
        "sanity: Normal mode shows both"
    );

    ed.feed_key(key('i'));
    assert_eq!(
        eol_lines(&mut ed, pid),
        vec![1],
        "primary line 0 hidden, line 1 kept"
    );
}

#[test]
fn unflagged_eol_text_stays_on_the_insert_cursor_line() {
    let (mut ed, pid) = two_line_editor_with_eol(&[("lsp", 0, "e0", false)]);
    ed.feed_key(key('i'));
    assert_eq!(eol_lines(&mut ed, pid), vec![0]);
}

#[test]
fn a_hidden_entry_lets_another_source_show_on_that_line() {
    let (mut ed, pid) = two_line_editor_with_eol(&[
        ("a-flagged", 0, "hidden", true),
        ("z-plain", 0, "shown", false),
    ]);
    ed.feed_key(key('i'));
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(60, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let by_line = ed.state.panes.render.get(pid).unwrap().eol_text();
    let inserts = by_line
        .get(&hume_rope::line::ContentLine::new(0))
        .expect("the unflagged source still shows on line 0");
    assert_eq!(inserts[0].text, "shown");
}

#[test]
fn eol_text_reappears_after_leaving_insert() {
    let (mut ed, pid) = two_line_editor_with_eol(&[("lsp", 0, "e0", true)]);
    ed.feed_key(key('i'));
    assert_eq!(eol_lines(&mut ed, pid), Vec::<usize>::new());
    ed.feed_key(key_esc());
    assert_eq!(eol_lines(&mut ed, pid), vec![0]);
}
