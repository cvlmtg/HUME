// Diagnostics end-of-line inline summary: the render-provider half that runs
// on every platform. The Steel-driven summary/overlay tests (which load the
// real `core:lsp` plugin) live in `unix/lsp_diagnostics_inline.rs`.

use super::*;
use crate::editor::decorations::InlineDiagnosticEntry;
use hume_engine::pipeline::RenderContext;

/// `update_inline_diagnostics_providers` (`lifecycle.rs`) must hand the full,
/// untruncated message through to the pane's `InlineInsert` — the per-line
/// summary text set via `set-inline-diagnostics!` must reach the render
/// provider byte-for-byte. (`format_buffer_line`'s trailing-insert path then
/// splits this `InlineInsert` into one cell per grapheme so a terminal
/// flush doesn't clobber it past the first column — covered directly by
/// `format::tests::trailing_insert_emits_one_cell_per_grapheme` in
/// `hume-engine`, since a ratatui `Buffer` snapshot here can't observe that
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
    ed.state.decorations.set_inline_diagnostics(
        bid,
        vec![InlineDiagnosticEntry {
            line: 0,
            text: message.to_string(),
            scope: "diagnostic.error".to_string(),
        }],
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(60, 8, &mut ctx);
    let pid = ed.state.focused_pane_id;
    let by_line = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .inline_diagnostics
        .read()
        .unwrap();
    let inserts = by_line.get(&0).expect("line 0 must have an insert");
    assert_eq!(inserts.len(), 1);
    assert_eq!(
        inserts[0].text, message,
        "the full message must reach the provider, not a prefix of it"
    );
}
