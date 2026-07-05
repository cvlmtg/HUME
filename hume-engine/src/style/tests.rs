    use super::*;
    use crate::theme::Theme;
    use crate::types::{CellContent, DisplayRow, Grapheme, ResolvedStyle, RowKind, Selection};
    use std::collections::HashMap;

    /// Test driver mirroring the live pipeline's Style-stage orchestration
    /// (`pipeline.rs::render_buffer_line`): primary-based `is_head_line`,
    /// `rebuild_tier_bufs` once per buffer line, `style_row` per display row.
    /// No highlight providers or tree — these tests cover cursor/selection styling only.
    fn apply_styles(
        rows: &[DisplayRow],
        graphemes: &[Grapheme],
        selections: &[Selection],
        mode: EditorMode,
        theme: &Theme,
        rope: &ropey::Rope,
        scratch: &mut StyleScratch,
    ) {
        scratch.populate_sorted_sels(selections, 0);
        scratch
            .styles
            .resize(graphemes.len(), ResolvedStyle::default());
        let mut current_line: Option<usize> = None;
        for row in rows {
            let Some(line_idx) = row.kind.line_idx() else {
                continue; // virtual row: styles stay default
            };
            if current_line != Some(line_idx) {
                current_line = Some(line_idx);
                rebuild_tier_bufs(line_idx, None, &[], rope, scratch);
            }
            let line_start_char = rope.line_to_char(line_idx);
            let line_end_char = rope.line_to_char(line_idx + 1);
            let is_head_line = scratch
                .primary_idx_in_sorted
                .and_then(|i| scratch.sorted_sels.get(i))
                .is_some_and(|s| s.head >= line_start_char && s.head < line_end_char);
            style_row(
                row,
                graphemes,
                line_start_char,
                line_end_char,
                is_head_line,
                mode,
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
                col: i as u16,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            })
            .collect()
    }

    fn make_row(graphemes: std::ops::Range<usize>) -> DisplayRow {
        DisplayRow {
            kind: RowKind::LineStart { line_idx: 0 },
            graphemes,
        }
    }

    fn default_theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn no_selections_yields_default_style() {
        let rope = ropey::Rope::from_str("abc");
        let graphemes = make_graphemes(3);
        let rows = vec![make_row(0..3)];
        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &[],
            EditorMode::Normal,
            &default_theme(),
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

    #[test]
    fn selection_head_overrides_default() {
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        let selections = vec![Selection { anchor: 2, head: 2 }];

        // Theme with a cursor style so we can detect the override.
        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        // Grapheme at col 2 (index 2) should have the cursor style.
        assert_eq!(scratch.styles[2].fg, Some(ratatui::style::Color::Red));
        // Other graphemes should not.
        assert_eq!(scratch.styles[0].fg, None);
    }

    /// Build graphemes for "hello\n": 5 content graphemes + 1 eol sentinel.
    fn make_graphemes_with_sentinel() -> Vec<Grapheme> {
        let mut gs = (0..5usize)
            .map(|i| Grapheme {
                byte_range: i..i + 1,
                char_offset: i,
                col: i as u16,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            })
            .collect::<Vec<_>>();
        // eol sentinel at char_offset=5, col=5 (the `\n` position).
        gs.push(Grapheme {
            byte_range: 5..5,
            char_offset: 5,
            col: 5,
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
        let rows = vec![make_row(0..6)]; // all 6 graphemes in one row

        let mut styles_map = std::collections::HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        // Line selection: anchor=0, head=5 (the '\n').
        let selections = vec![Selection { anchor: 0, head: 5 }];
        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        // The eol sentinel at index 5 must have the cursor style.
        assert_eq!(
            scratch.styles[5].fg,
            Some(ratatui::style::Color::Red),
            "eol sentinel (head on \\n) must receive cursor styling"
        );
        // The 'o' grapheme (index 4) must NOT have cursor styling (it's in selection, not head).
        assert_ne!(
            scratch.styles[4].fg,
            Some(ratatui::style::Color::Red),
            "grapheme before \\n must not have cursor styling"
        );
    }

    #[test]
    fn selection_range_highlighted() {
        // Graphemes at cols 0,1,2. Selection spans chars 1..3 (cols 1 and 2).
        let rope = ropey::Rope::from_str("abc");
        let graphemes = make_graphemes(3);
        let rows = vec![make_row(0..3)];
        let selections = vec![Selection { anchor: 1, head: 3 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(scratch.styles[0].bg, None, "col 0 outside selection");
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Red),
            "col 1 inside selection"
        );
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Red),
            "col 2 inside selection"
        );
    }

    /// Regression test: backward selections (head < anchor, e.g. after flip-selections)
    /// must highlight their full inclusive range. Before the fix, the anchor cell at
    /// the high end of the range was excluded from the selection span and rendered plain.
    #[test]
    fn backward_selection_anchor_cell_highlighted() {
        // "foo": chars 0,1,2. Backward selection: head=0, anchor=2.
        // Expected: col 0 painted as cursor (head), cols 1 and 2 painted as selection.
        let rope = ropey::Rope::from_str("foo");
        let graphemes = make_graphemes(3);
        let rows = vec![make_row(0..3)];
        let selections = vec![Selection { anchor: 2, head: 0 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::White),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::White),
            "col 0 is the head — must have cursor fg"
        );
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Blue),
            "col 1 is inside selection — must have selection bg"
        );
        // Regression: col 2 is the anchor (highest char), was rendered plain before fix.
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Blue),
            "col 2 is the anchor — must have selection bg (regression)"
        );
    }

    /// Regression: a collapsed selection (anchor == head, i.e. bare cursor) must
    /// not emit a selection-highlight span. In Insert mode the bar cursor is
    /// transparent, so a spurious 1-cell span shows through as a highlighted char.
    #[test]
    fn insert_mode_collapsed_selection_not_highlighted() {
        let rope = ropey::Rope::from_str("foo");
        let graphemes = make_graphemes(3);
        let rows = vec![make_row(0..3)];
        // Collapsed selection: head == anchor == char 1 (the 'o').
        let selections = vec![Selection { anchor: 1, head: 1 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Insert,
            &theme,
            &rope,
            &mut scratch,
        );

        // The cursor cell itself carries Tier-0 cursor styling, not selection bg.
        assert_ne!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Blue),
            "col 1 is the collapsed cursor — must NOT have selection bg"
        );
        // Neighboring cells are also not highlighted.
        assert_ne!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Blue),
            "col 0 not highlighted"
        );
        assert_ne!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Blue),
            "col 2 not highlighted"
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
            col: 0,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        };
        let g1 = Grapheme {
            byte_range: 1..2,
            char_offset: 1,
            col: 1,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        };
        let g2 = Grapheme {
            byte_range: 0..1,
            char_offset: 3,
            col: 0,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        };
        let g3 = Grapheme {
            byte_range: 1..2,
            char_offset: 4,
            col: 1,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        };
        let graphemes = vec![g0, g1, g2, g3];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..2,
            },
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 1 },
                graphemes: 2..4,
            },
        ];
        let selections = vec![Selection { anchor: 0, head: 0 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursorline",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Green),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Green),
            "line 0 has cursorline bg"
        );
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Green),
            "line 0 has cursorline bg"
        );
        assert_eq!(scratch.styles[2].bg, None, "line 1 has no cursorline bg");
        assert_eq!(scratch.styles[3].bg, None, "line 1 has no cursorline bg");
    }

    #[test]
    fn insert_mode_uses_insert_cursor_scope() {
        let rope = ropey::Rope::from_str("ab");
        let graphemes = make_graphemes(2);
        let rows = vec![make_row(0..2)];
        let selections = vec![Selection { anchor: 0, head: 0 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor.insert",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Green),
                ..Default::default()
            },
        );
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Insert,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::Green),
            "Insert uses ui.cursor.insert scope"
        );
    }

    #[test]
    fn insert_head_is_transparent_without_insert_scope() {
        // Theme defines ui.cursor with a block bg but NOT ui.cursor.insert.
        // In Insert mode the head cell must NOT inherit the block bg so the real
        // terminal bar cursor shows through.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        // Two selections: head 0 = primary, head 2 = secondary.
        let selections = vec![
            Selection { anchor: 0, head: 0 },
            Selection { anchor: 2, head: 2 },
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Insert,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].bg, None,
            "primary insert head has no block bg"
        );
        assert_eq!(
            scratch.styles[2].bg, None,
            "secondary insert head has no block bg"
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
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 0..1,
                char_offset: 2,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 0..1,
                char_offset: 4,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..1,
            },
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 1 },
                graphemes: 1..2,
            },
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 2 },
                graphemes: 2..3,
            },
        ];
        let selections = vec![
            Selection { anchor: 0, head: 0 },
            Selection { anchor: 4, head: 4 },
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursorline",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Blue),
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
    fn virtual_rows_keep_default_style() {
        let rope = ropey::Rope::from_str("ab");
        let graphemes = vec![
            Grapheme {
                byte_range: 0..1,
                char_offset: 0,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 0..0,
                char_offset: usize::MAX,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Virtual { start: 0, len: 4 },
                indent_depth: 0,
                scope: None,
            },
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..1,
            },
            DisplayRow {
                kind: RowKind::Virtual {
                    provider_id: 0,
                    anchor_line: 0,
                },
                graphemes: 1..2,
            },
        ];
        let selections = vec![Selection { anchor: 0, head: 0 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursorline",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        // Virtual row grapheme stays at default style.
        assert_eq!(scratch.styles[1], ResolvedStyle::default());
    }

    // ── Primary vs secondary selection head ─────────────────────────────────

    #[test]
    fn primary_head_gets_primary_style() {
        // Two selection heads on the same line (cols 0 and 2). Primary is first in the
        // selections slice (col 0). Theme has distinct styles for primary vs secondary.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        let selections = vec![
            Selection { anchor: 0, head: 0 }, // primary (col 0)
            Selection { anchor: 2, head: 2 }, // secondary (col 2)
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor.primary",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Yellow),
                ..Default::default()
            },
        );
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::Yellow),
            "primary head gets ui.cursor.primary"
        );
        assert_eq!(
            scratch.styles[2].fg,
            Some(ratatui::style::Color::Red),
            "secondary head gets ui.cursor"
        );
        assert_eq!(scratch.styles[1].fg, None, "non-head grapheme unchanged");
    }

    #[test]
    fn primary_selection_gets_primary_style() {
        // Two selections on the same line. Primary is first (bytes 0..2), secondary is bytes 3..5.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        let selections = vec![
            Selection { anchor: 0, head: 2 }, // primary
            Selection { anchor: 3, head: 5 }, // secondary
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection.primary",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Cyan),
                ..Default::default()
            },
        );
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        // Primary selection: cols 0 and 1 (bytes 0..2)
        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Cyan),
            "col 0 in primary selection"
        );
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Cyan),
            "col 1 in primary selection"
        );
        // Secondary selection: cols 3 and 4 (bytes 3..5)
        assert_eq!(
            scratch.styles[3].bg,
            Some(ratatui::style::Color::Blue),
            "col 3 in secondary selection"
        );
        assert_eq!(
            scratch.styles[4].bg,
            Some(ratatui::style::Color::Blue),
            "col 4 in secondary selection"
        );
        // Col 2 is the head of the primary selection — included in the span, so it gets primary bg.
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Cyan),
            "col 2 is primary head — must have primary selection bg"
        );
    }

    #[test]
    fn primary_head_falls_back_when_no_primary_scope() {
        // Theme does not define ui.cursor.primary — both heads should get ui.cursor.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        let selections = vec![
            Selection { anchor: 0, head: 0 }, // primary
            Selection { anchor: 2, head: 2 }, // secondary
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        // Both heads get ui.cursor via dot-notation fallback.
        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::Red),
            "primary falls back to ui.cursor"
        );
        assert_eq!(
            scratch.styles[2].fg,
            Some(ratatui::style::Color::Red),
            "secondary uses ui.cursor"
        );
    }

    #[test]
    fn head_on_wrapped_line_only_on_correct_segment() {
        // Simulate a wrapped line: line 0 has two display rows.
        // First segment: graphemes at byte ranges 0..1 (col 0), 1..2 (col 1), 2..3 (col 2).
        // Second segment: graphemes at byte ranges 3..4 (col 0), 4..5 (col 1).
        // Cursor head is at char_offset=1 (first segment). It must appear only on row 0.
        // "abcde" has no newlines so all chars are on line 0 with absolute char offsets 0..5.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = vec![
            Grapheme {
                byte_range: 0..1,
                char_offset: 0,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 1..2,
                char_offset: 1,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 2..3,
                char_offset: 2,
                col: 2,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 3..4,
                char_offset: 3,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            }, // wrap segment
            Grapheme {
                byte_range: 4..5,
                char_offset: 4,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..3,
            },
            DisplayRow {
                kind: RowKind::Wrap {
                    line_idx: 0,
                    wrap_row: 1,
                },
                graphemes: 3..5,
            },
        ];
        let selections = vec![Selection { anchor: 1, head: 1 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        // Selection head at byte 1 → col 1 in the first segment.
        assert_eq!(
            scratch.styles[1].fg,
            Some(ratatui::style::Color::Red),
            "selection head at col 1 in first segment"
        );
        // Second segment graphemes must NOT have the head style.
        assert_eq!(
            scratch.styles[3].fg, None,
            "wrap segment col 0 must not show head style"
        );
        assert_eq!(
            scratch.styles[4].fg, None,
            "wrap segment col 1 must not show head style"
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
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 1..2,
                char_offset: 1,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 2..3,
                char_offset: 2,
                col: 2,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 3..4,
                char_offset: 3,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 4..5,
                char_offset: 4,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..3,
            },
            DisplayRow {
                kind: RowKind::Wrap {
                    line_idx: 0,
                    wrap_row: 1,
                },
                graphemes: 3..5,
            },
        ];
        let selections = vec![Selection { anchor: 0, head: 2 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        // Segment 0: cols 0 and 1 should be highlighted (selection spans bytes 0..2).
        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Blue),
            "col 0 in selection"
        );
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Blue),
            "col 1 in selection"
        );
        // Col 2 is the head of the selection (char 2 is included in [0,2]); it gets selection bg.
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Blue),
            "col 2 is selection head — included in inclusive span"
        );
        // Segment 1: no selection highlight at all.
        assert_eq!(
            scratch.styles[3].bg, None,
            "wrap segment col 0 must not show selection"
        );
        assert_eq!(
            scratch.styles[4].bg, None,
            "wrap segment col 1 must not show selection"
        );
    }

    // ── Inline-insert scope styling (B3) ─────────────────────────────────

    #[test]
    fn inline_insert_scope_is_layered_but_neighbour_is_not() {
        // Insert with an interned scope mapped to fg: Red. The insert cell's
        // resolved style must carry that scope; the real grapheme next to it
        // must not.
        let rope = ropey::Rope::from_str("ab");
        let mut registry = crate::theme::ScopeRegistry::new();
        let hint_scope = registry.intern("hint");
        let inserts = vec![crate::providers::InlineInsert {
            byte_offset: 0,
            text: "H".into(),
            scope: hint_scope,
        }];
        let mut fmt = crate::format::FormatScratch::new();
        crate::format::format_buffer_line(
            &rope,
            0,
            4,
            &crate::pane::WhitespaceConfig::default(),
            &crate::pane::WrapMode::None,
            None,
            &inserts,
            &mut fmt,
        );

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "hint",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let mut theme = Theme::new(styles_map, ResolvedStyle::default());
        theme.bake(&registry);

        let mut scratch = StyleScratch::new();
        apply_styles(
            &fmt.display_rows,
            &fmt.graphemes,
            &[],
            EditorMode::Normal,
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
            Some(ratatui::style::Color::Red),
            "insert cell must carry its own scope's style"
        );
        assert_eq!(
            scratch.styles[a_idx].fg, None,
            "neighbouring real grapheme must not inherit the insert's scope"
        );
    }

    // ── Inline-insert char_offset partition invariant (B2) ────────────────

    /// Drive the real formatter with a mid-row insert, then style the result —
    /// end-to-end coverage that `resolve_grapheme_col`'s partition_point lands
    /// on the real grapheme, not the insert sharing its char_offset.
    #[test]
    fn insert_mid_row_head_resolves_to_real_grapheme_col() {
        // "abcdef", width-2 insert before 'c' (byte offset 2). Layout by hand:
        // a(col0) b(col1) [insert XY](col2..4) c(col4) d(col5) e(col6) f(col7).
        // The insert and 'c' share char_offset 2 (the insert is pushed first,
        // at the offset of the grapheme it precedes) — the exact tie
        // `resolve_grapheme_col` must break in favour of the real grapheme.
        // Cursor at char 2 ('c') must land at col 4, not the insert's col 2.
        let rope = ropey::Rope::from_str("abcdef");
        let mut registry = crate::theme::ScopeRegistry::new();
        let insert_scope = registry.intern("test");
        let inserts = vec![crate::providers::InlineInsert {
            byte_offset: 2,
            text: "XY".into(),
            scope: insert_scope,
        }];
        let mut fmt = crate::format::FormatScratch::new();
        crate::format::format_buffer_line(
            &rope,
            0,
            4,
            &crate::pane::WhitespaceConfig::default(),
            &crate::pane::WrapMode::None,
            None,
            &inserts,
            &mut fmt,
        );

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let mut theme = Theme::new(styles_map, ResolvedStyle::default());
        theme.bake(&registry);
        let selections = vec![Selection { anchor: 2, head: 2 }];
        let mut scratch = StyleScratch::new();
        apply_styles(
            &fmt.display_rows,
            &fmt.graphemes,
            &selections,
            EditorMode::Normal,
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
            fmt.graphemes[c_idx].col, 4,
            "'c' shifts right by the insert's width"
        );
        assert_eq!(
            scratch.styles[c_idx].fg,
            Some(ratatui::style::Color::Red),
            "cursor head must land on 'c', not the insert sharing its char_offset"
        );

        let insert_idx = fmt
            .graphemes
            .iter()
            .position(|g| matches!(g.content, CellContent::Virtual { .. }))
            .expect("insert grapheme present");
        assert_ne!(
            scratch.styles[insert_idx].fg,
            Some(ratatui::style::Color::Red),
            "the insert cell itself must not receive cursor styling"
        );
    }

    #[test]
    fn selection_spanning_row_start_insert_begins_at_first_real_grapheme() {
        // Insert at byte 0 — the row starts with a virtual cell at col 0,
        // then 'a' at col 1, 'b' at col 2, etc. A selection over chars 0..1
        // ('a','b') must start its highlighted span at 'a's col (1), not the
        // insert's col (0).
        let rope = ropey::Rope::from_str("abcdef");
        let mut registry = crate::theme::ScopeRegistry::new();
        let insert_scope = registry.intern("test");
        let inserts = vec![crate::providers::InlineInsert {
            byte_offset: 0,
            text: "Z".into(),
            scope: insert_scope,
        }];
        let mut fmt = crate::format::FormatScratch::new();
        crate::format::format_buffer_line(
            &rope,
            0,
            4,
            &crate::pane::WhitespaceConfig::default(),
            &crate::pane::WrapMode::None,
            None,
            &inserts,
            &mut fmt,
        );

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let mut theme = Theme::new(styles_map, ResolvedStyle::default());
        theme.bake(&registry);
        let selections = vec![Selection { anchor: 0, head: 1 }]; // 'a' and 'b'
        let mut scratch = StyleScratch::new();
        apply_styles(
            &fmt.display_rows,
            &fmt.graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        let insert_idx = fmt
            .graphemes
            .iter()
            .position(|g| matches!(g.content, CellContent::Virtual { .. }))
            .expect("insert grapheme present");
        assert_eq!(fmt.graphemes[insert_idx].col, 0);
        assert_eq!(
            scratch.styles[insert_idx].bg, None,
            "the row-start insert cell must not be painted as part of the selection"
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
        assert_eq!(fmt.graphemes[a_idx].col, 1);
        assert_eq!(
            scratch.styles[a_idx].bg,
            Some(ratatui::style::Color::Blue),
            "'a' is the first real grapheme — selection span must start here"
        );
        assert_eq!(scratch.styles[b_idx].bg, Some(ratatui::style::Color::Blue));
    }
