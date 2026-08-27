// EOL text: the render-provider half that runs on every platform. The
// Steel-driven summary/overlay tests (which load the real `core:lsp` plugin)
// live in `unix/lsp_diagnostics_inline.rs`.

use super::*;
use crate::editor::decorations::EolTextEntry;
use hume_engine::pipeline::RenderContext;

/// `update_eol_text_providers` (`decoration_providers.rs`) must hand the full,
/// untruncated message through to the pane's `InlineInsert` — the per-line
/// summary text set via `set-eol-text!` must reach the render provider
/// byte-for-byte. (`format_buffer_line`'s trailing-insert path then splits
/// this `InlineInsert` into one cell per grapheme so a terminal flush
/// doesn't clobber it past the first column — covered directly by
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
    ed.state.config.decorations.set_eol_text(
        "lsp".to_string(),
        bid,
        vec![EolTextEntry {
            pos: 0,
            text: message.to_string(),
            scope: "diagnostic.error".to_string(),
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(60, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focused_pane_id;
    let by_line = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .eol_text
        .read()
        .unwrap();
    let inserts = by_line.get(&0).expect("line 0 must have an insert");
    assert_eq!(inserts.len(), 1);
    assert_eq!(
        inserts[0].text, message,
        "the full message must reach the provider, not a prefix of it"
    );
}

/// Two entries from the *same* source landing on the same line — the shape a
/// remap produces when an edit collapses several originally-distinct lines
/// into one; `last_writer_per_line` folds them, keeping the last. Before
/// this fix, `update_eol_text_providers` pushed onto a per-line `Vec`
/// instead of folding, so both entries survived and rendered concatenated at
/// the same byte offset.
#[test]
fn two_entries_from_one_source_on_the_same_line_collapse_to_the_last_one() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "let x = 1".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let bid = ed.focused_buffer_id();
    ed.state.config.decorations.set_eol_text(
        "diagnostics".to_string(),
        bid,
        vec![
            EolTextEntry {
                pos: 0,
                text: "first".to_string(),
                scope: "diagnostic.error".to_string(),
            },
            EolTextEntry {
                pos: 0,
                text: "second".to_string(),
                scope: "diagnostic.warning".to_string(),
            },
        ],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(60, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focused_pane_id;
    let by_line = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .eol_text
        .read()
        .unwrap();
    let inserts = by_line.get(&0).expect("line 0 must have an insert");
    assert_eq!(
        inserts.len(),
        1,
        "two same-line entries from one source must collapse to one insert, \
         not stack"
    );
    assert_eq!(
        inserts[0].text, "second",
        "the later entry must win — last_writer_per_line folds left-to-right"
    );
}

/// Two sources tinting the same line — the cross-source
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
    ed.state.config.decorations.set_eol_text(
        "z-plugin".to_string(),
        bid,
        vec![EolTextEntry {
            pos: 0,
            text: "from-z".to_string(),
            scope: "diagnostic.error".to_string(),
        }],
    );
    ed.state.config.decorations.set_eol_text(
        "a-plugin".to_string(),
        bid,
        vec![EolTextEntry {
            pos: 0,
            text: "from-a".to_string(),
            scope: "diagnostic.error".to_string(),
        }],
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(60, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focused_pane_id;
    let by_line = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .eol_text
        .read()
        .unwrap();
    let inserts = by_line.get(&0).expect("line 0 must have an insert");
    assert_eq!(inserts.len(), 1);
    assert_eq!(
        inserts[0].text, "from-a",
        "the alphabetically first source (\"a-plugin\") must win"
    );
}
