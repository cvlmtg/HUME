// The conceal mirror: `update_conceal_providers` flattens every source's
// `decorations.conceals` entries into each pane's `PaneConceals` map. Then
// the lines each pane's selections touch, which it draws raw.

use super::*;
use hume_decorations::{ConcealEntry, ConcealEntrySpan};
use hume_engine::pipeline::RenderContext;
use hume_engine::providers::{Conceal, ConcealSpan, ConcealText};
use hume_grid::Rect;
use hume_rope::line::ContentLine;
use hume_rope::offset::{ExclusiveRange, InclusiveRange};

fn label(text: &str) -> ConcealText {
    ConcealText {
        text: text.into(),
        scope: None,
    }
}

/// A `Hidden` entry over chars `start..end` drawing `text` in its place.
pub(super) fn hidden(start: usize, end: usize, text: &str) -> ConcealEntry {
    ConcealEntry {
        span: ConcealEntrySpan::Hidden {
            start: co(start),
            end: co(end),
            text: Some(label(text)),
        },
        reveal: None,
    }
}

/// A `Hidden` entry over chars `start..end` with nothing drawn in its place.
pub(super) fn hiding(start: usize, end: usize) -> ConcealEntry {
    ConcealEntry {
        span: ConcealEntrySpan::Hidden {
            start: co(start),
            end: co(end),
            text: None,
        },
        reveal: None,
    }
}

fn inserted(at: usize, text: &str) -> ConcealEntry {
    ConcealEntry {
        span: ConcealEntrySpan::Inserted {
            at: co(at),
            text: label(text),
        },
        reveal: None,
    }
}

pub(super) fn revealing(mut entry: ConcealEntry, start: usize, end: usize) -> ConcealEntry {
    entry.reveal = Some(ExclusiveRange::new(co(start), co(end)));
    entry
}

const VIEW: Rect = Rect::new(0, 0, 80, 25);

fn draw(ed: &mut Editor) {
    ed.resize_terminal(80, 25);
    ed.settle();
    ed.prepare_frame(&mut RenderContext::new());
}

/// What the focused pane's mirror holds for `line`: each conceal's byte
/// range, its text, and its reveal lines.
fn mirrored(
    ed: &Editor,
    line: usize,
) -> Vec<(
    ExclusiveRange<usize>,
    String,
    Option<InclusiveRange<ContentLine>>,
)> {
    let pid = ed.view.focused_pane();
    let by_line = ed.state.panes.entries[pid].render.conceals();
    let concealed: &[Conceal] = by_line
        .get(&ContentLine::new(line))
        .map_or(&[], Vec::as_slice);
    concealed
        .iter()
        .map(|c| {
            let text = match &c.span {
                ConcealSpan::Hidden { text, .. } => text.as_ref(),
                ConcealSpan::Inserted { text, .. } => Some(text),
            };
            let bytes = c.bytes();
            (
                ExclusiveRange::new(bytes.start.index(), bytes.end.index()),
                text.map(|t| t.text.to_string()).unwrap_or_default(),
                c.reveal,
            )
        })
        .collect()
}

pub(super) fn set(ed: &mut Editor, source: &str, entries: Vec<ConcealEntry>) {
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .decorations
        .set_conceals(source.to_string(), bid, entries);
}

fn span(start: usize, end: usize) -> ExclusiveRange<usize> {
    ExclusiveRange::new(start, end)
}

#[test]
fn cross_source_overlap_keeps_the_alphabetically_first_source() {
    let mut ed = editor_from("-[*]>*bold**\n");
    set(&mut ed, "b", vec![hidden(0, 2, "B")]);
    set(&mut ed, "a", vec![hidden(1, 3, "A")]);
    draw(&mut ed);

    assert_eq!(mirrored(&ed, 0), vec![(span(1, 3), "A".into(), None)]);
}

#[test]
fn later_source_conceal_strictly_containing_an_earlier_one_is_dropped_whole() {
    let mut ed = editor_from("-[*]>*bold**\n");
    set(&mut ed, "a", vec![hidden(2, 4, "A")]);
    set(&mut ed, "b", vec![hidden(0, 8, "B")]);
    draw(&mut ed);

    assert_eq!(mirrored(&ed, 0), vec![(span(2, 4), "A".into(), None)]);
}

#[test]
fn two_sources_inserted_conceals_at_one_position_keep_the_first_source_only() {
    let mut ed = editor_from("-[*]>*bold**\n");
    set(&mut ed, "b", vec![inserted(0, "B")]);
    set(&mut ed, "a", vec![inserted(0, "A")]);
    draw(&mut ed);

    assert_eq!(mirrored(&ed, 0), vec![(span(0, 0), "A".into(), None)]);
}

#[test]
fn line_joined_after_a_table_keeps_its_last_row_in_the_reveal_span() {
    // "|a|\n|b|\nx\n": the table is lines 0 and 1, the reveal span runs from
    // char 0 to the start of line 2 (char 8).
    let mut ed = editor_from("-[|]>a|\n|b|\nx\n");
    set(
        &mut ed,
        "table",
        vec![
            revealing(hidden(0, 1, "["), 0, 8),
            revealing(hidden(4, 5, "["), 0, 8),
        ],
    );
    draw(&mut ed);
    let lines = Some(InclusiveRange::new(
        ContentLine::new(0),
        ContentLine::new(1),
    ));
    assert_eq!(
        mirrored(&ed, 1),
        vec![(span(0, 1), "[".into(), lines)],
        "sanity: the span covers both rows before the edit"
    );

    // Delete the `\n` at char 7: line 2's `x` joins the last row, and the
    // span's end (8) moves to 7, still past that row's start.
    select(&mut ed, &[(7, 7)], 0);
    ed.feed_key(key('d'));
    draw(&mut ed);

    assert_eq!(ed.doc().text().to_string(), "|a|\n|b|x\n");
    assert_eq!(mirrored(&ed, 0), vec![(span(0, 1), "[".into(), lines)]);
    assert_eq!(mirrored(&ed, 1), vec![(span(0, 1), "[".into(), lines)]);
}

// Revealing the lines selections touch

/// An editor on `marked`'s text and selections, opened as startup opens one
/// (`open_headless`), so its pane has full chrome and its decoration
/// providers, which `editor_from`'s pane lacks. The text reaches it through
/// a file that lives as long as the returned guard.
pub(super) fn decorated_editor(marked: &str) -> (Editor, tempfile::TempPath) {
    let (text, _) = parse_state(marked);
    let (path, file) = temp_file(&text.to_string());
    let mut ed = open_headless(Some(path));
    select_marked(&mut ed, marked);
    (ed, file)
}

/// Rows `0..n` of pane `pid` on a fresh frame.
fn drawn_rows(ed: &mut Editor, pid: PaneId, n: u16) -> Vec<String> {
    let buf = ed.render_to_buf(VIEW);
    super::scope_headers::pane_rows(ed, &buf, pid, n)
}

#[test]
fn cursor_move_reveals_without_touching_the_mirror_or_generation() {
    let (mut ed, _file) = decorated_editor("-[x]>\n**bold**\n");
    set(&mut ed, "md", vec![hiding(2, 4), hiding(8, 10)]);
    let pid = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    assert_eq!(
        drawn_rows(&mut ed, pid, 2),
        ["x", "bold"],
        "premise: line 1 draws concealed"
    );
    let generation = ed.state.config.decorations.generation(bid);
    let mirror = mirrored(&ed, 1);

    ed.feed_key(key('j'));

    assert_eq!(drawn_rows(&mut ed, pid, 2), ["x", "**bold**"]);
    assert_eq!(ed.state.config.decorations.generation(bid), generation);
    assert_eq!(mirrored(&ed, 1), mirror);
}

#[test]
fn two_panes_on_one_buffer_reveal_independently() {
    let (mut ed, _file) = decorated_editor("-[*]>*a**\n**b**\n");
    set(
        &mut ed,
        "md",
        vec![hiding(0, 2), hiding(3, 5), hiding(6, 8), hiding(9, 11)],
    );
    let a = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let b = ed.view.focused_pane();
    select(&mut ed, &[(6, 6)], 0);

    assert_eq!(drawn_rows(&mut ed, a, 2), ["**a**", "b"]);
    assert_eq!(drawn_rows(&mut ed, b, 2), ["a", "**b**"]);
}

#[test]
fn ctrl_d_in_a_concealed_buffer_leaves_the_view_settled() {
    let (mut ed, _file) = decorated_editor(&format!("-[0]>{}", &numbered_lines(100)[1..]));
    pin_no_wrap(&mut ed);
    set(&mut ed, "md", vec![hiding(0, 1)]);
    let pid = ed.view.focused_pane();
    draw(&mut ed);

    ed.feed_key(key_ctrl('d'));

    assert_ne!(
        ed.current_view().primary().lines().start,
        ContentLine::new(0),
        "premise: the cursor moved to another line"
    );
    let fresh = ed.state.layout_key(&ed.view, pid, ed.pane_size(pid));
    let pbs = ed.state.panes.entries.shown(&ed.view.panes, pid);
    assert!(pbs.last_layout_key == Some(fresh));
    assert!(!pbs.reveal_pending);
}

#[test]
fn a_buffer_without_conceals_lays_out_the_same_drawn_and_raw() {
    let (ed, _file) = decorated_editor("0\n1\n2\n-[3]>\n4\n");
    let pid = ed.view.focused_pane();
    let key = ed.state.layout_key(&ed.view, pid, ed.pane_size(pid));

    assert_eq!(key.inputs(), key.raw_inputs());
}

#[test]
fn click_on_a_heading_glyph_lands_on_the_hash() {
    let (mut ed, _file) = decorated_editor("-[x]>\n# Title\n");
    set(&mut ed, "md", vec![hidden(2, 4, "H1 ")]);
    let pid = ed.view.focused_pane();
    let buf = ed.render_to_buf(VIEW);
    assert_eq!(
        super::scope_headers::pane_rows(&ed, &buf, pid, 2),
        ["x", "H1 Title"],
        "premise: the glyph stands in for `# `"
    );
    let (rect, gutter) = super::scope_headers::drawn_pane(&ed, &buf, pid);

    // Column 2 is the glyph's last cell; raw, it would be the `T`.
    ed.handle_input(mouse_left_down(rect.x + gutter + 2, rect.y + 1));

    assert_eq!(ed.current_view().primary().head().offset(), co(2));
}
