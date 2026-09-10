use super::*;
use crate::providers::ProviderSet;
use crate::test_support::{bg, fg, theme_with};
use crate::theme::Theme;
use crate::types::{CellContent, DisplayLine, DisplayLineKind, Grapheme, ResolvedStyle, Selection};
use hume_grid::Rgb;
use hume_rope::column::DisplayLineCol;
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::offset::CharOffset;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

fn dc(n: u32) -> DisplayLineCol {
    DisplayLineCol::new(n)
}

/// Test driver mirroring the live pipeline's ResolvedStyle-stage orchestration
/// (`pipeline::pane_render::render_pane`'s display-line walk): primary-based
/// `is_head_line`, `rebuild_line_decorations` once per buffer line,
/// `style_display_line` per display line.
/// No highlight providers or tree — these tests cover cursor/selection styling only.
#[allow(clippy::too_many_arguments)] // mirrors the live pipeline stage's own arity
fn apply_styles(
    lines: &[DisplayLine],
    graphemes: &[Grapheme],
    selections: &[Selection],
    mode: EditorMode,
    cursor_is_block: bool,
    theme: &Theme,
    rope: &ropey::Rope,
    scratch: &mut StyleScratch,
) {
    scratch.populate_sorted_sels(selections, 0);
    scratch
        .styles
        .resize(graphemes.len(), ResolvedStyle::default());
    let mut current_line: Option<hume_rope::line::RopeyLine> = None;
    let mut tint = None;
    for dline in lines {
        let Some(line_idx) = dline.kind.line_idx() else {
            continue; // virtual display line: styles stay default
        };
        if current_line != Some(line_idx) {
            current_line = Some(line_idx);
            // Trusted mint, not `RopeyLine::to_content`: these fixture lines
            // are hand-built with small real-content indices, and several
            // fixture ropes here don't even uphold the trailing-newline
            // invariant `to_content` would check.
            let content_line = ContentLine::new(line_idx.index());
            tint = rebuild_line_decorations(content_line, None, &ProviderSet::new(), rope, scratch);
        }
        let line_start_char = co(hume_rope::lines::line_start_char(rope, line_idx).index());
        let line_end_char = hume_rope::lines::next_line_start(rope, line_idx);
        let is_head_line = scratch
            .primary_idx_in_sorted
            .and_then(|i| scratch.sorted_sels.get(i))
            .is_some_and(|s| s.head >= line_start_char && s.head < line_end_char);
        style_display_line(
            dline,
            graphemes,
            line_start_char,
            line_end_char,
            is_head_line,
            tint,
            mode,
            cursor_is_block,
            theme,
            scratch,
        );
    }
}

fn make_graphemes(count: usize) -> Vec<Grapheme> {
    (0..count)
        .map(|i| Grapheme {
            byte_range: i..i + 1,
            char_offset: i,
            display_col: dc(i as u32),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        })
        .collect()
}

fn make_display_line(graphemes: std::ops::Range<usize>) -> DisplayLine {
    DisplayLine {
        kind: DisplayLineKind::LineStart {
            line_idx: RopeyLine::new(0),
        },
        graphemes,
    }
}

#[test]
fn no_selections_yields_default_style() {
    let rope = ropey::Rope::from_str("abc");
    let graphemes = make_graphemes(3);
    let lines = vec![make_display_line(0..3)];
    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &[],
        EditorMode::Normal,
        true,
        &Theme::default(),
        &rope,
        &mut scratch,
    );

    assert_eq!(scratch.styles.len(), 3);
    assert!(
        scratch
            .styles
            .iter()
            .all(|s| *s == ResolvedStyle::default())
    );
}

/// A `line_tint` scope with `fg`/modifiers as well as `bg` must only
/// contribute its `bg` — the row-fill paint site (`pane_render.rs`'s
/// `row_bg`) can only ever read a background color out of the same scope, so
/// a fg/modifier applied here would show up on content cells but nowhere the
/// row-fill site paints (gutter, trailing fill past end-of-line),
/// contradicting a kind documented as a full-row *background* tint.
///
/// Fail oracle: layering the tint's whole `ResolvedStyle` (the pre-fix
/// behavior) makes this fail on both the fg and the modifier assertions.
#[test]
fn line_tint_applies_only_background_not_fg_or_modifiers() {
    let graphemes = make_graphemes(3);
    let lines = [make_display_line(0..3)];
    let mut scratch = StyleScratch::new();

    let mut registry = crate::theme::ScopeRegistry::new();
    let tint_scope = registry.intern("diff.plus.line");
    let mut theme = crate::test_support::theme_with([(
        "diff.plus.line",
        ResolvedStyle {
            fg: Some(Rgb(255, 0, 0)),
            bg: Some(Rgb(0, 255, 0)),
            modifiers: crate::types::Modifiers::BOLD,
            ..Default::default()
        },
    )]);
    theme.bake(&registry);

    scratch.populate_sorted_sels(&[], 0);
    scratch
        .styles
        .resize(graphemes.len(), ResolvedStyle::default());
    style_display_line(
        &lines[0],
        &graphemes,
        co(0),
        co(3),
        false, // not the cursor line — isolates the tint's own contribution
        Some(tint_scope),
        EditorMode::Normal,
        true,
        &theme,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].bg,
        Some(Rgb(0, 255, 0)),
        "the tint's background must still apply"
    );
    assert_eq!(
        scratch.styles[0].fg, None,
        "the tint's foreground must not apply — only the row-fill paint \
         site's `.bg` can express this decoration"
    );
    assert_eq!(
        scratch.styles[0].modifiers,
        crate::types::Modifiers::empty(),
        "the tint's modifiers must not apply, same reasoning as fg"
    );
}

#[test]
fn selection_head_overrides_default() {
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = make_graphemes(5);
    let lines = vec![make_display_line(0..5)];
    let selections = vec![Selection {
        anchor: co(2),
        head: co(2),
    }];

    // Theme with a cursor style so we can detect the override.
    let theme = theme_with([("ui.cursor", fg(Rgb(255, 0, 0)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    // Grapheme at display_col 2 (index 2) should have the cursor style.
    assert_eq!(scratch.styles[2].fg, Some(Rgb(255, 0, 0)));
    // Other graphemes should not.
    assert_eq!(scratch.styles[0].fg, None);
}

/// Build graphemes for "hello\n": 5 content graphemes + 1 eol sentinel.
fn make_graphemes_with_sentinel() -> Vec<Grapheme> {
    let mut gs = (0..5usize)
        .map(|i| Grapheme {
            byte_range: i..i + 1,
            char_offset: i,
            display_col: dc(i as u32),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        })
        .collect::<Vec<_>>();
    // eol sentinel at char_offset=5, display_col=5 (the `\n` position).
    gs.push(Grapheme {
        byte_range: 5..5,
        char_offset: 5,
        display_col: dc(5),
        width: 1,
        content: CellContent::Empty,
        indent_depth: 0,
        scope: None,
    });
    gs
}

/// After `x` (select-line), the selection head lands on the `\n` char.
/// The eol sentinel grapheme must receive cursor styling so the cursor is visible.
#[test]
fn selection_head_on_newline_is_visible() {
    let rope = ropey::Rope::from_str("hello\n");
    let graphemes = make_graphemes_with_sentinel();
    let lines = vec![make_display_line(0..6)]; // all 6 graphemes in one display line

    let theme = theme_with([("ui.cursor", fg(Rgb(255, 0, 0)))]);

    // Line selection: anchor=0, head=5 (the '\n').
    let selections = vec![Selection {
        anchor: co(0),
        head: co(5),
    }];
    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    // The eol sentinel at index 5 must have the cursor style.
    assert_eq!(
        scratch.styles[5].fg,
        Some(Rgb(255, 0, 0)),
        "eol sentinel (head on \\n) must receive cursor styling"
    );
    // The 'o' grapheme (index 4) must NOT have cursor styling (it's in selection, not head).
    assert_ne!(
        scratch.styles[4].fg,
        Some(Rgb(255, 0, 0)),
        "grapheme before \\n must not have cursor styling"
    );
}

#[test]
fn selection_range_highlighted() {
    // Graphemes at cols 0,1,2. Selection spans chars 1..3 (cols 1 and 2).
    let rope = ropey::Rope::from_str("abc");
    let graphemes = make_graphemes(3);
    let lines = vec![make_display_line(0..3)];
    let selections = vec![Selection {
        anchor: co(1),
        head: co(3),
    }];

    let theme = theme_with([("ui.selection", bg(Rgb(255, 0, 0)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].bg, None,
        "display_col 0 outside selection"
    );
    assert_eq!(
        scratch.styles[1].bg,
        Some(Rgb(255, 0, 0)),
        "display_col 1 inside selection"
    );
    assert_eq!(
        scratch.styles[2].bg,
        Some(Rgb(255, 0, 0)),
        "display_col 2 inside selection"
    );
}

/// Regression test: backward selections (head < anchor, e.g. after flip-selections)
/// must highlight their full inclusive range. Before the fix, the anchor cell at
/// the high end of the range was excluded from the selection span and rendered plain.
#[test]
fn backward_selection_anchor_cell_highlighted() {
    // "foo": chars 0,1,2. Backward selection: head=0, anchor=2.
    // Expected: display_col 0 painted as cursor (head), cols 1 and 2 painted as selection.
    let rope = ropey::Rope::from_str("foo");
    let graphemes = make_graphemes(3);
    let lines = vec![make_display_line(0..3)];
    let selections = vec![Selection {
        anchor: co(2),
        head: co(0),
    }];

    let theme = theme_with([
        ("ui.selection", bg(Rgb(0, 0, 255))),
        ("ui.cursor", fg(Rgb(255, 255, 255))),
    ]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].fg,
        Some(Rgb(255, 255, 255)),
        "display_col 0 is the head — must have cursor fg"
    );
    assert_eq!(
        scratch.styles[1].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 1 is inside selection — must have selection bg"
    );
    // Regression: display_col 2 is the anchor (highest char), was rendered plain before fix.
    assert_eq!(
        scratch.styles[2].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 2 is the anchor — must have selection bg (regression)"
    );
}

/// Regression: a collapsed selection (anchor == head, i.e. bare cursor) must
/// not emit a selection-highlight span — a bare cursor marks a position, not a
/// one-character selection.
#[test]
fn insert_mode_collapsed_selection_not_highlighted() {
    let rope = ropey::Rope::from_str("foo");
    let graphemes = make_graphemes(3);
    let lines = vec![make_display_line(0..3)];
    // Collapsed selection: head == anchor == char 1 (the 'o').
    let selections = vec![Selection {
        anchor: co(1),
        head: co(1),
    }];

    let theme = theme_with([("ui.selection", bg(Rgb(0, 0, 255)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Insert,
        false,
        &theme,
        &rope,
        &mut scratch,
    );

    // The cursor cell itself carries Tier-0 cursor styling, not selection bg.
    assert_ne!(
        scratch.styles[1].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 1 is the collapsed cursor — must NOT have selection bg"
    );
    // Neighboring cells are also not highlighted.
    assert_ne!(
        scratch.styles[0].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 0 not highlighted"
    );
    assert_ne!(
        scratch.styles[2].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 2 not highlighted"
    );
}

#[test]
fn cursorline_background_applied_to_cursor_line_only() {
    // Two lines; cursor on line 0.
    // "ab\ncd": a=char0, b=char1, \n=char2, c=char3, d=char4
    let rope = ropey::Rope::from_str("ab\ncd");
    let g0 = Grapheme {
        byte_range: 0..1,
        char_offset: 0,
        display_col: dc(0),
        width: 1,
        content: crate::types::CellContent::Grapheme,
        indent_depth: 0,
        scope: None,
    };
    let g1 = Grapheme {
        byte_range: 1..2,
        char_offset: 1,
        display_col: dc(1),
        width: 1,
        content: crate::types::CellContent::Grapheme,
        indent_depth: 0,
        scope: None,
    };
    let g2 = Grapheme {
        byte_range: 0..1,
        char_offset: 3,
        display_col: dc(0),
        width: 1,
        content: crate::types::CellContent::Grapheme,
        indent_depth: 0,
        scope: None,
    };
    let g3 = Grapheme {
        byte_range: 1..2,
        char_offset: 4,
        display_col: dc(1),
        width: 1,
        content: crate::types::CellContent::Grapheme,
        indent_depth: 0,
        scope: None,
    };
    let graphemes = vec![g0, g1, g2, g3];
    let lines = vec![
        DisplayLine {
            kind: DisplayLineKind::LineStart {
                line_idx: RopeyLine::new(0),
            },
            graphemes: 0..2,
        },
        DisplayLine {
            kind: DisplayLineKind::LineStart {
                line_idx: RopeyLine::new(1),
            },
            graphemes: 2..4,
        },
    ];
    let selections = vec![Selection {
        anchor: co(0),
        head: co(0),
    }];

    let theme = theme_with([("ui.cursorline", bg(Rgb(0, 255, 0)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].bg,
        Some(Rgb(0, 255, 0)),
        "line 0 has cursorline bg"
    );
    assert_eq!(
        scratch.styles[1].bg,
        Some(Rgb(0, 255, 0)),
        "line 0 has cursorline bg"
    );
    assert_eq!(scratch.styles[2].bg, None, "line 1 has no cursorline bg");
    assert_eq!(scratch.styles[3].bg, None, "line 1 has no cursorline bg");
}

/// Regression: everforest defines `ui.cursor.insert` (the secondary scope)
/// and `ui.cursor` (the plain block scope), but no `ui.cursor.primary.insert`
/// or `ui.cursor.primary`. A block-shape primary head must land on the plain
/// `ui.cursor` colour — the bug this ladder replaced gave it the *secondary*
/// insert colour instead, making both heads identical.
#[test]
fn insert_mode_block_primary_head_never_uses_the_secondary_insert_scope() {
    let rope = ropey::Rope::from_str("ab");
    let graphemes = make_graphemes(2);
    let lines = vec![make_display_line(0..2)];
    let selections = vec![Selection {
        anchor: co(0),
        head: co(0),
    }];

    let theme = theme_with([
        ("ui.cursor.insert", fg(Rgb(0, 255, 0))),
        ("ui.cursor", fg(Rgb(255, 0, 0))),
    ]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Insert,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].fg,
        Some(Rgb(255, 0, 0)),
        "block-shape primary head uses ui.cursor, never the secondary ui.cursor.insert scope"
    );
}

/// Pins the non-block fall-through this project departs from Helix on — see
/// the Tier 1/0 comment in `style_display_line` for the why: a collapsed secondary
/// head goes bare, a ranged one falls through to plain `ui.selection`.
#[test]
fn insert_bar_shape_hides_both_heads_and_keeps_selection_styling() {
    let rope = ropey::Rope::from_str("abcdefg");
    let graphemes = make_graphemes(7);
    let lines = vec![make_display_line(0..7)];
    // head 0 = primary (collapsed), head 4 = secondary (ranged, anchor 2..4),
    // head 6 = secondary (collapsed).
    let selections = vec![
        Selection {
            anchor: co(0),
            head: co(0),
        },
        Selection {
            anchor: co(2),
            head: co(4),
        },
        Selection {
            anchor: co(6),
            head: co(6),
        },
    ];

    let theme = theme_with([
        ("ui.cursor.insert", fg(Rgb(0, 255, 0))),
        ("ui.cursor", fg(Rgb(255, 0, 0))),
        ("ui.selection", bg(Rgb(0, 0, 255))),
    ]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Insert,
        false,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].fg, None,
        "primary head unpainted under the default Bar shape"
    );
    assert_eq!(
        scratch.styles[4].bg,
        Some(Rgb(0, 0, 255)),
        "ranged secondary's head cell falls through to ui.selection, not ui.cursor.insert"
    );
    assert_eq!(
        scratch.styles[6].fg, None,
        "collapsed secondary head has nothing to fall through to — stays bare"
    );
}

#[test]
fn cursorline_applies_only_to_primary_head_line() {
    // Two selection heads on lines 0 and 2; line 1 should not get cursorline.
    // "a\nb\nc": a=char0, \n=char1, b=char2, \n=char3, c=char4
    let rope = ropey::Rope::from_str("a\nb\nc");
    let graphemes = vec![
        Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            display_col: dc(0),
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 0..1,
            char_offset: 2,
            display_col: dc(0),
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 0..1,
            char_offset: 4,
            display_col: dc(0),
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
    ];
    let lines = vec![
        DisplayLine {
            kind: DisplayLineKind::LineStart {
                line_idx: RopeyLine::new(0),
            },
            graphemes: 0..1,
        },
        DisplayLine {
            kind: DisplayLineKind::LineStart {
                line_idx: RopeyLine::new(1),
            },
            graphemes: 1..2,
        },
        DisplayLine {
            kind: DisplayLineKind::LineStart {
                line_idx: RopeyLine::new(2),
            },
            graphemes: 2..3,
        },
    ];
    let selections = vec![
        Selection {
            anchor: co(0),
            head: co(0),
        },
        Selection {
            anchor: co(4),
            head: co(4),
        },
    ];

    let theme = theme_with([("ui.cursorline", bg(Rgb(0, 0, 255)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].bg,
        Some(Rgb(0, 0, 255)),
        "line 0 head line"
    );
    assert_eq!(scratch.styles[1].bg, None, "line 1 no head line");
    // line 2 has a non-primary selection head: primary-based is_head_line = false,
    // so the live pipeline does NOT apply cursorline there.
    assert_eq!(
        scratch.styles[2].bg, None,
        "line 2 non-primary head: no cursorline"
    );
}

#[test]
fn virtual_display_lines_keep_default_style() {
    let rope = ropey::Rope::from_str("ab");
    let graphemes = vec![
        Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            display_col: dc(0),
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 0..0,
            char_offset: usize::MAX,
            display_col: dc(0),
            width: 1,
            content: crate::types::CellContent::Virtual { start: 0, len: 4 },
            indent_depth: 0,
            scope: None,
        },
    ];
    let lines = vec![
        DisplayLine {
            kind: DisplayLineKind::LineStart {
                line_idx: RopeyLine::new(0),
            },
            graphemes: 0..1,
        },
        DisplayLine {
            kind: DisplayLineKind::Virtual {
                provider_id: 0,
                anchor_line: RopeyLine::new(0),
            },
            graphemes: 1..2,
        },
    ];
    let selections = vec![Selection {
        anchor: co(0),
        head: co(0),
    }];

    let theme = theme_with([("ui.cursorline", bg(Rgb(0, 0, 255)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    // Virtual display line grapheme stays at default style.
    assert_eq!(scratch.styles[1], ResolvedStyle::default());
}

// ── Primary vs secondary selection head ─────────────────────────────────

#[test]
fn primary_head_gets_primary_style() {
    // Two selection heads on the same line (cols 0 and 2). Primary is first in the
    // selections slice (display_col 0). Theme has distinct styles for primary vs secondary.
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = make_graphemes(5);
    let lines = vec![make_display_line(0..5)];
    let selections = vec![
        Selection {
            anchor: co(0),
            head: co(0),
        }, // primary (display_col 0)
        Selection {
            anchor: co(2),
            head: co(2),
        }, // secondary (display_col 2)
    ];

    let theme = theme_with([
        ("ui.cursor.primary", fg(Rgb(255, 255, 0))),
        ("ui.cursor", fg(Rgb(255, 0, 0))),
    ]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].fg,
        Some(Rgb(255, 255, 0)),
        "primary head gets ui.cursor.primary"
    );
    assert_eq!(
        scratch.styles[2].fg,
        Some(Rgb(255, 0, 0)),
        "secondary head gets ui.cursor"
    );
    assert_eq!(scratch.styles[1].fg, None, "non-head grapheme unchanged");
}

#[test]
fn primary_selection_gets_primary_style() {
    // Two selections on the same line. Primary is first (bytes 0..2), secondary is bytes 3..5.
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = make_graphemes(5);
    let lines = vec![make_display_line(0..5)];
    let selections = vec![
        Selection {
            anchor: co(0),
            head: co(2),
        }, // primary
        Selection {
            anchor: co(3),
            head: co(5),
        }, // secondary
    ];

    let theme = theme_with([
        ("ui.selection.primary", bg(Rgb(0, 255, 255))),
        ("ui.selection", bg(Rgb(0, 0, 255))),
    ]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    // Primary selection: cols 0 and 1 (bytes 0..2)
    assert_eq!(
        scratch.styles[0].bg,
        Some(Rgb(0, 255, 255)),
        "display_col 0 in primary selection"
    );
    assert_eq!(
        scratch.styles[1].bg,
        Some(Rgb(0, 255, 255)),
        "display_col 1 in primary selection"
    );
    // Secondary selection: cols 3 and 4 (bytes 3..5)
    assert_eq!(
        scratch.styles[3].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 3 in secondary selection"
    );
    assert_eq!(
        scratch.styles[4].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 4 in secondary selection"
    );
    // Col 2 is the head of the primary selection, painted (Block shape) from
    // the cursor ladder rather than the selection tier. With no ui.cursor*
    // scope defined at all, that ladder's tail is the bare `ui.selection`,
    // not `ui.selection.primary` — matching Helix's own `base_cursor_scope`,
    // which only ever falls to `ui.selection`, never the `.primary` variant.
    assert_eq!(
        scratch.styles[2].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 2 is the primary head — falls to the bare ui.selection, not .primary"
    );
}

#[test]
fn extend_mode_uses_select_cursor_scope() {
    // Extend is HUME's name for Helix's Select mode — a theme's
    // ui.cursor.select / ui.cursor.primary.select must apply there, not the
    // plain block scope.
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = make_graphemes(5);
    let lines = vec![make_display_line(0..5)];
    let selections = vec![
        Selection {
            anchor: co(0),
            head: co(0),
        }, // primary
        Selection {
            anchor: co(2),
            head: co(2),
        }, // secondary
    ];

    let theme = theme_with([
        ("ui.cursor", fg(Rgb(255, 0, 0))),
        ("ui.cursor.primary", fg(Rgb(0, 255, 0))),
        ("ui.cursor.select", fg(Rgb(0, 0, 255))),
        ("ui.cursor.primary.select", fg(Rgb(255, 255, 0))),
    ]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Extend,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].fg,
        Some(Rgb(255, 255, 0)),
        "primary head in Extend mode gets ui.cursor.primary.select"
    );
    assert_eq!(
        scratch.styles[2].fg,
        Some(Rgb(0, 0, 255)),
        "secondary head in Extend mode gets ui.cursor.select"
    );
}

/// Insert-mode twin of the Extend test above. Both heads get their own
/// distinct exact-match colour, with the primary painted (`Block` shape) so
/// its own colour is observable at all.
#[test]
fn insert_mode_distinguishes_primary_from_secondary_head() {
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = make_graphemes(5);
    let lines = vec![make_display_line(0..5)];
    let selections = vec![
        Selection {
            anchor: co(0),
            head: co(0),
        }, // primary
        Selection {
            anchor: co(2),
            head: co(2),
        }, // secondary
    ];

    let theme = theme_with([
        ("ui.cursor.insert", fg(Rgb(0, 0, 255))),
        ("ui.cursor.primary.insert", fg(Rgb(255, 255, 0))),
    ]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Insert,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].fg,
        Some(Rgb(255, 255, 0)),
        "primary head in Insert mode gets ui.cursor.primary.insert"
    );
    assert_eq!(
        scratch.styles[2].fg,
        Some(Rgb(0, 0, 255)),
        "secondary head in Insert mode gets ui.cursor.insert"
    );
}

/// HUME's Command/Search/Select prompt modes have no Helix equivalent — Helix
/// keeps the underlying document mode while a prompt is open, and these
/// prompts have no cursor-shape option of their own — so document heads use
/// the plain Normal ladder, never the Insert one.
#[test]
fn prompt_modes_use_the_normal_cursor_scope_for_document_heads() {
    let rope = ropey::Rope::from_str("ab");
    let graphemes = make_graphemes(2);
    let lines = vec![make_display_line(0..2)];
    let selections = vec![Selection {
        anchor: co(0),
        head: co(0),
    }];

    let theme = theme_with([
        ("ui.cursor.primary", fg(Rgb(255, 0, 0))),
        ("ui.cursor.primary.insert", fg(Rgb(255, 255, 0))),
    ]);

    for mode in [EditorMode::Command, EditorMode::Search, EditorMode::Sift] {
        let mut scratch = StyleScratch::new();
        apply_styles(
            &lines,
            &graphemes,
            &selections,
            mode,
            true,
            &theme,
            &rope,
            &mut scratch,
        );
        assert_eq!(
            scratch.styles[0].fg,
            Some(Rgb(255, 0, 0)),
            "{mode:?} document head uses ui.cursor.primary, never the insert scope"
        );
    }
}

/// Helix's own carve-out for a non-block primary head with a real selection:
/// a forward selection leaves the head cell bare (the real terminal cursor is
/// the sole indicator there), but a reverse one (head before anchor) keeps
/// the selection tier on the head cell.
#[test]
fn insert_mode_bar_primary_head_geometry_depends_on_selection_direction() {
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = make_graphemes(5);

    let theme = theme_with([("ui.selection.primary", bg(Rgb(255, 0, 255)))]);

    // Forward: anchor 0, head 3 — head cell (col 3) is left bare.
    let lines = vec![make_display_line(0..5)];
    let selections = vec![Selection {
        anchor: co(0),
        head: co(3),
    }];
    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Insert,
        false,
        &theme,
        &rope,
        &mut scratch,
    );
    assert_eq!(
        scratch.styles[2].bg,
        Some(Rgb(255, 0, 255)),
        "forward: col 2 (inside the span, not the head) keeps the selection bg"
    );
    assert_eq!(
        scratch.styles[3].bg, None,
        "forward: col 3 (the head) is left bare under a non-block shape"
    );

    // Reverse: anchor 3, head 0 — head cell (col 0) keeps the selection bg.
    let selections = vec![Selection {
        anchor: co(3),
        head: co(0),
    }];
    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Insert,
        false,
        &theme,
        &rope,
        &mut scratch,
    );
    assert_eq!(
        scratch.styles[0].bg,
        Some(Rgb(255, 0, 255)),
        "reverse: col 0 (the head) keeps the selection bg"
    );
}

#[test]
fn normal_mode_still_uses_plain_cursor_scope_not_select() {
    // Regression guard: adding Extend-mode select scopes must not leak
    // into Normal mode.
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = make_graphemes(5);
    let lines = vec![make_display_line(0..5)];
    let selections = vec![Selection {
        anchor: co(0),
        head: co(0),
    }];

    let theme = theme_with([
        ("ui.cursor.primary", fg(Rgb(0, 255, 0))),
        ("ui.cursor.primary.select", fg(Rgb(255, 255, 0))),
    ]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.styles[0].fg,
        Some(Rgb(0, 255, 0)),
        "Normal mode must use ui.cursor.primary, not the select variant"
    );
}

#[test]
fn primary_head_falls_back_when_no_primary_scope() {
    // Theme does not define ui.cursor.primary — both heads should get ui.cursor.
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = make_graphemes(5);
    let lines = vec![make_display_line(0..5)];
    let selections = vec![
        Selection {
            anchor: co(0),
            head: co(0),
        }, // primary
        Selection {
            anchor: co(2),
            head: co(2),
        }, // secondary
    ];

    let theme = theme_with([("ui.cursor", fg(Rgb(255, 0, 0)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    // Both heads get ui.cursor via dot-notation fallback.
    assert_eq!(
        scratch.styles[0].fg,
        Some(Rgb(255, 0, 0)),
        "primary falls back to ui.cursor"
    );
    assert_eq!(
        scratch.styles[2].fg,
        Some(Rgb(255, 0, 0)),
        "secondary uses ui.cursor"
    );
}

#[test]
fn head_on_wrapped_line_only_on_correct_segment() {
    // Simulate a wrapped line: line 0 has two display lines.
    // First segment: graphemes at byte ranges 0..1 (display_col 0), 1..2 (display_col 1), 2..3 (display_col 2).
    // Second segment: graphemes at byte ranges 3..4 (display_col 0), 4..5 (display_col 1).
    // Cursor head is at char_offset=1 (first segment). It must appear only on display line 0.
    // "abcde" has no newlines so all chars are on line 0 with absolute char offsets 0..5.
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = vec![
        Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            display_col: dc(0),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 1..2,
            char_offset: 1,
            display_col: dc(1),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 2..3,
            char_offset: 2,
            display_col: dc(2),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 3..4,
            char_offset: 3,
            display_col: dc(0),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        }, // wrap segment
        Grapheme {
            byte_range: 4..5,
            char_offset: 4,
            display_col: dc(1),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
    ];
    let lines = vec![
        DisplayLine {
            kind: DisplayLineKind::LineStart {
                line_idx: RopeyLine::new(0),
            },
            graphemes: 0..3,
        },
        DisplayLine {
            kind: DisplayLineKind::Wrap {
                line_idx: RopeyLine::new(0),
                wrap_index: 1,
            },
            graphemes: 3..5,
        },
    ];
    let selections = vec![Selection {
        anchor: co(1),
        head: co(1),
    }];

    let theme = theme_with([("ui.cursor", fg(Rgb(255, 0, 0)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    // Selection head at byte 1 → display_col 1 in the first segment.
    assert_eq!(
        scratch.styles[1].fg,
        Some(Rgb(255, 0, 0)),
        "selection head at display_col 1 in first segment"
    );
    // Second segment graphemes must NOT have the head style.
    assert_eq!(
        scratch.styles[3].fg, None,
        "wrap segment display_col 0 must not show head style"
    );
    assert_eq!(
        scratch.styles[4].fg, None,
        "wrap segment display_col 1 must not show head style"
    );
}

#[test]
fn selection_on_wrapped_line_does_not_highlight_other_segments() {
    // Same wrapped line layout as head_on_wrapped_line_only_on_correct_segment.
    // A selection spanning chars 0..2 (cols 0–1 in segment 0) must not
    // produce a selection highlight on segment 1 at all.
    let rope = ropey::Rope::from_str("abcde");
    let graphemes = vec![
        Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            display_col: dc(0),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 1..2,
            char_offset: 1,
            display_col: dc(1),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 2..3,
            char_offset: 2,
            display_col: dc(2),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 3..4,
            char_offset: 3,
            display_col: dc(0),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 4..5,
            char_offset: 4,
            display_col: dc(1),
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
    ];
    let lines = vec![
        DisplayLine {
            kind: DisplayLineKind::LineStart {
                line_idx: RopeyLine::new(0),
            },
            graphemes: 0..3,
        },
        DisplayLine {
            kind: DisplayLineKind::Wrap {
                line_idx: RopeyLine::new(0),
                wrap_index: 1,
            },
            graphemes: 3..5,
        },
    ];
    let selections = vec![Selection {
        anchor: co(0),
        head: co(2),
    }];

    let theme = theme_with([("ui.selection", bg(Rgb(0, 0, 255)))]);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &lines,
        &graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    // Segment 0: cols 0 and 1 should be highlighted (selection spans bytes 0..2).
    assert_eq!(
        scratch.styles[0].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 0 in selection"
    );
    assert_eq!(
        scratch.styles[1].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 1 in selection"
    );
    // Col 2 is the head of the selection (char 2 is included in [0,2]); it gets selection bg.
    assert_eq!(
        scratch.styles[2].bg,
        Some(Rgb(0, 0, 255)),
        "display_col 2 is selection head — included in inclusive span"
    );
    // Segment 1: no selection highlight at all.
    assert_eq!(
        scratch.styles[3].bg, None,
        "wrap segment display_col 0 must not show selection"
    );
    assert_eq!(
        scratch.styles[4].bg, None,
        "wrap segment display_col 1 must not show selection"
    );
}

// ── Inline-insert scope styling ─────────────────────────────────────────

#[test]
fn inline_insert_scope_is_layered_but_neighbour_is_not() {
    // Insert with an interned scope mapped to fg: Red. The insert cell's
    // resolved style must carry that scope; the real grapheme next to it
    // must not.
    let rope = ropey::Rope::from_str("ab");
    let mut registry = crate::theme::ScopeRegistry::new();
    let hint_scope = registry.intern("hint");
    let inserts = vec![crate::providers::InlineInsert {
        byte_offset: hume_rope::column::ByteCol::new(0),
        text: "H".into(),
        scope: hint_scope,
    }];
    let mut fmt = crate::format::LineFormat::new();
    crate::format::format_buffer_line(
        &rope,
        RopeyLine::new(0),
        4,
        &crate::pane::WhitespaceConfig::default(),
        &crate::pane::WrapMode::None,
        None,
        crate::format::FormatBound::Full,
        &inserts,
        &mut fmt,
    );

    let mut theme = theme_with([("hint", fg(Rgb(255, 0, 0)))]);
    theme.bake(&registry);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &fmt.display_lines,
        &fmt.graphemes,
        &[],
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    let insert_idx = fmt
        .graphemes
        .iter()
        .position(|g| matches!(g.content, CellContent::Virtual { .. }))
        .expect("insert grapheme present");
    let a_idx = fmt
        .graphemes
        .iter()
        .position(|g| g.char_offset == 0 && matches!(g.content, CellContent::Grapheme))
        .expect("'a' grapheme present");

    assert_eq!(
        scratch.styles[insert_idx].fg,
        Some(Rgb(255, 0, 0)),
        "insert cell must carry its own scope's style"
    );
    assert_eq!(
        scratch.styles[a_idx].fg, None,
        "neighbouring real grapheme must not inherit the insert's scope"
    );
}

#[test]
fn an_invisible_cluster_is_styled_by_its_own_scope_not_the_text_around_it() {
    // The placeholder must not read as ordinary text: it carries
    // `ui.virtual.invisible` regardless of the syntax colour at that
    // position, which is what lets a theme make a bidi override catch the
    // eye. Its neighbours keep their own styling.
    let rope = ropey::Rope::from_str("a\u{202E}b");
    let mut registry = crate::theme::ScopeRegistry::new();
    registry.intern("ui.virtual.invisible");
    let mut fmt = crate::format::LineFormat::new();
    crate::format::format_buffer_line(
        &rope,
        RopeyLine::new(0),
        4,
        &crate::pane::WhitespaceConfig::default(),
        &crate::pane::WrapMode::None,
        None,
        crate::format::FormatBound::Full,
        &[],
        &mut fmt,
    );

    let mut theme = theme_with([("ui.virtual.invisible", fg(Rgb(255, 0, 0)))]);
    theme.bake(&registry);

    let mut scratch = StyleScratch::new();
    apply_styles(
        &fmt.display_lines,
        &fmt.graphemes,
        &[],
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    let placeholder_idx = fmt
        .graphemes
        .iter()
        .position(|g| matches!(g.content, CellContent::Placeholder { .. }))
        .expect("the bidi override must produce a placeholder cell");
    assert_eq!(
        scratch.styles[placeholder_idx].fg,
        Some(Rgb(255, 0, 0)),
        "the placeholder must carry ui.virtual.invisible"
    );
    let a_idx = fmt
        .graphemes
        .iter()
        .position(|g| g.char_offset == 0 && matches!(g.content, CellContent::Grapheme))
        .expect("'a' grapheme present");
    assert_eq!(
        scratch.styles[a_idx].fg, None,
        "the text around it keeps its own styling"
    );
}

#[test]
fn a_whitespace_indicator_is_styled_by_its_own_scope_not_the_text_around_it() {
    // An opted-in whitespace glyph must carry `ui.virtual.whitespace`
    // regardless of the syntax colour at that position — the same
    // contract `ui.virtual.invisible` gets for placeholders, above.
    let rope = ropey::Rope::from_str("a \tb");
    let ws = crate::pane::WhitespaceConfig {
        space: crate::pane::WhitespaceRender::All,
        tab: crate::pane::WhitespaceRender::All,
        ..crate::pane::WhitespaceConfig::default()
    };
    let mut fmt = crate::format::LineFormat::new();
    crate::format::format_buffer_line(
        &rope,
        RopeyLine::new(0),
        4,
        &ws,
        &crate::pane::WrapMode::None,
        None,
        crate::format::FormatBound::Full,
        &[],
        &mut fmt,
    );

    let mut theme = theme_with([("ui.virtual.whitespace", fg(Rgb(0, 255, 0)))]);
    theme.bake(&crate::theme::ScopeRegistry::new());

    let mut scratch = StyleScratch::new();
    apply_styles(
        &fmt.display_lines,
        &fmt.graphemes,
        &[],
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    let whitespace_idx = fmt
        .graphemes
        .iter()
        .position(|g| matches!(g.content, CellContent::Whitespace { .. }))
        .expect("the space indicator must produce a Whitespace cell");
    assert_eq!(
        scratch.styles[whitespace_idx].fg,
        Some(Rgb(0, 255, 0)),
        "the indicator must carry ui.virtual.whitespace"
    );
    let a_idx = fmt
        .graphemes
        .iter()
        .position(|g| g.char_offset == 0 && matches!(g.content, CellContent::Grapheme))
        .expect("'a' grapheme present");
    assert_eq!(
        scratch.styles[a_idx].fg, None,
        "the text around it keeps its own styling"
    );
}

#[test]
fn tab_fill_does_not_carry_the_whitespace_scope_when_its_indicator_is_off() {
    // A tab drawn as blank spaces (indicator off) must not pick up
    // `ui.virtual.whitespace` — that scope belongs only to the glyph the
    // user opted into, never to the fallback fill a theme's `bg` would
    // otherwise leak onto every tab expansion regardless of the setting.
    let rope = ropey::Rope::from_str("a\tb");
    let ws = crate::pane::WhitespaceConfig {
        tab: crate::pane::WhitespaceRender::None,
        ..crate::pane::WhitespaceConfig::default()
    };
    let mut fmt = crate::format::LineFormat::new();
    crate::format::format_buffer_line(
        &rope,
        RopeyLine::new(0),
        4,
        &ws,
        &crate::pane::WrapMode::None,
        None,
        crate::format::FormatBound::Full,
        &[],
        &mut fmt,
    );

    let mut theme = theme_with([("ui.virtual.whitespace", fg(Rgb(0, 255, 0)))]);
    theme.bake(&crate::theme::ScopeRegistry::new());

    let mut scratch = StyleScratch::new();
    apply_styles(
        &fmt.display_lines,
        &fmt.graphemes,
        &[],
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    let tab_fill_idx = fmt
        .graphemes
        .iter()
        .position(|g| matches!(g.content, CellContent::TabFill))
        .expect("the tab with its indicator off must produce a TabFill cell");
    assert_eq!(
        scratch.styles[tab_fill_idx].fg, None,
        "tab fill must not inherit ui.virtual.whitespace"
    );
}

// ── Inline-insert char_offset partition invariant ───────────────────────

/// Drive the real formatter with a mid-display-line insert, then style the result —
/// end-to-end coverage that `resolve_grapheme_display_col`'s partition_point lands
/// on the real grapheme, not the insert sharing its char_offset.
#[test]
fn insert_mid_display_line_head_resolves_to_real_grapheme_col() {
    // "abcdef", width-2 insert before 'c' (byte offset 2). Layout by hand:
    // a(col0) b(col1) [insert XY](col2..4) c(col4) d(col5) e(col6) f(col7).
    // The insert and 'c' share char_offset 2 (the insert is pushed first,
    // at the offset of the grapheme it precedes) — the exact tie
    // `resolve_grapheme_display_col` must break in favour of the real grapheme.
    // Cursor at char 2 ('c') must land at display_col 4, not the insert's display_col 2.
    let rope = ropey::Rope::from_str("abcdef");
    let mut registry = crate::theme::ScopeRegistry::new();
    let insert_scope = registry.intern("test");
    let inserts = vec![crate::providers::InlineInsert {
        byte_offset: hume_rope::column::ByteCol::new(2),
        text: "XY".into(),
        scope: insert_scope,
    }];
    let mut fmt = crate::format::LineFormat::new();
    crate::format::format_buffer_line(
        &rope,
        RopeyLine::new(0),
        4,
        &crate::pane::WhitespaceConfig::default(),
        &crate::pane::WrapMode::None,
        None,
        crate::format::FormatBound::Full,
        &inserts,
        &mut fmt,
    );

    let mut theme = theme_with([("ui.cursor", fg(Rgb(255, 0, 0)))]);
    theme.bake(&registry);
    let selections = vec![Selection {
        anchor: co(2),
        head: co(2),
    }];
    let mut scratch = StyleScratch::new();
    apply_styles(
        &fmt.display_lines,
        &fmt.graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    let c_idx = fmt
        .graphemes
        .iter()
        .position(|g| g.char_offset == 2 && matches!(g.content, CellContent::Grapheme))
        .expect("'c' grapheme present");
    assert_eq!(
        fmt.graphemes[c_idx].display_col,
        dc(4),
        "'c' shifts right by the insert's width"
    );
    assert_eq!(
        scratch.styles[c_idx].fg,
        Some(Rgb(255, 0, 0)),
        "cursor head must land on 'c', not the insert sharing its char_offset"
    );

    let insert_idx = fmt
        .graphemes
        .iter()
        .position(|g| matches!(g.content, CellContent::Virtual { .. }))
        .expect("insert grapheme present");
    assert_ne!(
        scratch.styles[insert_idx].fg,
        Some(Rgb(255, 0, 0)),
        "the insert cell itself must not receive cursor styling"
    );
}

#[test]
fn selection_spanning_display_line_start_insert_begins_at_first_real_grapheme() {
    // Insert at byte 0 — the display line starts with a virtual cell at display_col 0,
    // then 'a' at display_col 1, 'b' at display_col 2, etc. A selection over chars 0..1
    // ('a','b') must start its highlighted span at 'a's display_col (1), not the
    // insert's display_col (0).
    let rope = ropey::Rope::from_str("abcdef");
    let mut registry = crate::theme::ScopeRegistry::new();
    let insert_scope = registry.intern("test");
    let inserts = vec![crate::providers::InlineInsert {
        byte_offset: hume_rope::column::ByteCol::new(0),
        text: "Z".into(),
        scope: insert_scope,
    }];
    let mut fmt = crate::format::LineFormat::new();
    crate::format::format_buffer_line(
        &rope,
        RopeyLine::new(0),
        4,
        &crate::pane::WhitespaceConfig::default(),
        &crate::pane::WrapMode::None,
        None,
        crate::format::FormatBound::Full,
        &inserts,
        &mut fmt,
    );

    let mut theme = theme_with([("ui.selection", bg(Rgb(0, 0, 255)))]);
    theme.bake(&registry);
    let selections = vec![Selection {
        anchor: co(0),
        head: co(1),
    }]; // 'a' and 'b'
    let mut scratch = StyleScratch::new();
    apply_styles(
        &fmt.display_lines,
        &fmt.graphemes,
        &selections,
        EditorMode::Normal,
        true,
        &theme,
        &rope,
        &mut scratch,
    );

    let insert_idx = fmt
        .graphemes
        .iter()
        .position(|g| matches!(g.content, CellContent::Virtual { .. }))
        .expect("insert grapheme present");
    assert_eq!(fmt.graphemes[insert_idx].display_col, dc(0));
    assert_eq!(
        scratch.styles[insert_idx].bg, None,
        "the display-line-start insert cell must not be painted as part of the selection"
    );

    let a_idx = fmt
        .graphemes
        .iter()
        .position(|g| g.char_offset == 0 && matches!(g.content, CellContent::Grapheme))
        .expect("'a' grapheme present");
    let b_idx = fmt
        .graphemes
        .iter()
        .position(|g| g.char_offset == 1 && matches!(g.content, CellContent::Grapheme))
        .expect("'b' grapheme present");
    assert_eq!(fmt.graphemes[a_idx].display_col, dc(1));
    assert_eq!(
        scratch.styles[a_idx].bg,
        Some(Rgb(0, 0, 255)),
        "'a' is the first real grapheme — selection span must start here"
    );
    assert_eq!(scratch.styles[b_idx].bg, Some(Rgb(0, 0, 255)));
}
