use super::*;
use crate::theme::{ScopeRegistry, Theme};
use crate::types::EditorMode;

fn ctx(rope: &ropey::Rope) -> GutterRowCtx<'_> {
    GutterRowCtx {
        mode: EditorMode::Normal,
        primary_head_line: 0,
        rope,
    }
}

struct FixedSign {
    line: usize,
    sign: Sign,
}

impl SignSource for FixedSign {
    fn signs_for_line(&self, line_idx: usize, _ctx: &GutterRowCtx) -> Vec<Sign> {
        if line_idx == self.line {
            vec![self.sign.clone()]
        } else {
            Vec::new()
        }
    }
}

#[test]
fn higher_priority_sign_wins_on_the_same_line() {
    let mut registry = ScopeRegistry::new();
    let diag_scope = registry.intern("diagnostic");
    let git_scope = registry.intern("git");
    let blank_scope = registry.intern("ui.linenr");

    let mut lane = SignColumn::new(blank_scope);
    lane.add_source(Box::new(FixedSign {
        line: 3,
        sign: Sign {
            text: "!".into(),
            scope: diag_scope,
            priority: 10,
        },
    }));
    lane.add_source(Box::new(FixedSign {
        line: 3,
        sign: Sign {
            text: "+".into(),
            scope: git_scope,
            priority: 5,
        },
    }));

    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 3 }, &ctx(&rope))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "!", "priority 10 beats priority 5");
    assert_eq!(cell.scope, diag_scope);
}

#[test]
fn removing_the_winner_reveals_the_next_highest() {
    let mut registry = ScopeRegistry::new();
    let diag_scope = registry.intern("diagnostic");
    let git_scope = registry.intern("git");
    let blank_scope = registry.intern("ui.linenr");

    let mut lane = SignColumn::new(blank_scope);
    let winner_id = lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "!".into(),
            scope: diag_scope,
            priority: 10,
        },
    }));
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "+".into(),
            scope: git_scope,
            priority: 5,
        },
    }));

    assert!(lane.remove_source(winner_id));

    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 0 }, &ctx(&rope))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        cell.as_str(),
        "+",
        "with the priority-10 sign gone, 5 shows"
    );
}

#[test]
fn no_source_fires_renders_blank() {
    let mut registry = ScopeRegistry::new();
    let blank_scope = registry.intern("ui.linenr");
    let lane = SignColumn::new(blank_scope);
    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 0 }, &ctx(&rope))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), " ");
}

#[test]
fn sign_absent_on_wrap_virtual_and_filler_rows() {
    let mut registry = ScopeRegistry::new();
    let scope = registry.intern("diagnostic");
    let blank_scope = registry.intern("ui.linenr");
    let mut lane = SignColumn::new(blank_scope);
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "!".into(),
            scope,
            priority: 1,
        },
    }));
    let rope = ropey::Rope::new();

    for kind in [
        RowKind::Wrap {
            line_idx: 0,
            wrap_row: 1,
        },
        RowKind::Virtual {
            provider_id: 0,
            anchor_line: 0,
        },
        RowKind::Filler,
    ] {
        let cell = lane
            .render_row_cells(kind, &ctx(&rope))
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(cell.as_str(), " ", "{kind:?} must not show a sign");
    }
}

#[test]
fn width_is_configured_not_recomputed_per_frame() {
    let mut registry = ScopeRegistry::new();
    let blank_scope = registry.intern("ui.linenr");
    let lane = SignColumn::with_width(3, blank_scope);
    assert_eq!(lane.width(0), 3);
    assert_eq!(lane.width(999_999), 3, "stable regardless of file size");
}

#[test]
fn set_width_overrides_the_configured_width() {
    let mut registry = ScopeRegistry::new();
    let blank_scope = registry.intern("ui.linenr");
    let mut lane = SignColumn::with_width(2, blank_scope);
    lane.set_width(0);
    assert_eq!(lane.width(0), 0, "collapsed to zero when no signs exist");
    lane.set_width(2);
    assert_eq!(lane.width(0), 2, "restored once a sign exists again");
}

#[test]
fn sign_text_truncates_to_column_width_end_to_end() {
    // Full compose path (not just SignColumn::render_row in isolation):
    // a 3-glyph sign in a width-2 column must come out clipped by
    // `render::compose_gutter` (B8's gutter clipping), same as every
    // other gutter column — SignColumn adds no truncation of its own.
    let mut registry = ScopeRegistry::new();
    let scope = registry.intern("diagnostic");
    let blank_scope = registry.intern("ui.linenr");
    let mut lane = SignColumn::with_width(2, blank_scope); // usable = 1 cell
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "▶▶▶".into(),
            scope,
            priority: 1,
        },
    }));

    let graphemes = vec![crate::types::Grapheme {
        byte_range: 0..1,
        char_offset: 0,
        display_col: 0,
        width: 1,
        content: crate::types::CellContent::Grapheme,
        indent_depth: 0,
        scope: None,
    }];
    let rows = [crate::types::DisplayRow {
        kind: RowKind::LineStart { line_idx: 0 },
        graphemes: 0..1,
    }];
    let styles = vec![crate::types::ResolvedStyle::default()];
    let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> = vec![(0, Box::new(lane))];
    let visible = crate::layout::PaneGeometry {
        content_height: 1,
        content_width: 6,
        gutter_width: 2,
        last_line_idx: 0,
    };
    let viewport = crate::pane::ViewportState::new(8, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 1,
    };
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    let mut theme = Theme::default();
    theme.bake(&registry);
    let rope = ropey::Rope::from_str("x\n");
    let lane_widths = vec![2u16];
    let compose_ctx = crate::render::ComposeCtx {
        gutter_columns: &gutter_columns,
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        rope: &rope,
        default_gutter_scope: blank_scope,
    };
    let mut canvas = crate::render::Canvas::new(&mut buf, &theme, None);
    crate::render::compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "x",
        "",
        0,
        &lane_widths,
        &compose_ctx,
        &mut canvas,
        None,
    );

    let sym = |x: u16| {
        buf.cell(ratatui::layout::Position { x, y: 0 })
            .unwrap()
            .symbol()
            .to_string()
    };
    assert_eq!(sym(0), "▶", "only the first glyph of the sign fits");
    assert_eq!(sym(1), " ", "separator cell, not a straggler glyph");
    // Content area starts at x=2 (gutter_width) — must show the real
    // grapheme 'x', never a spillover from the sign text.
    assert_eq!(sym(2), "x");
}

/// A width-0 `SignColumn` (the auto-collapse state `set_width(0)`
/// produces when no sign exists for the pane's buffer) must render as if
/// it weren't registered at all — the gutter composer still iterates it,
/// but must not shift or corrupt whatever renders in the next column.
#[test]
fn zero_width_sign_column_leaves_the_next_column_untouched() {
    let mut registry = ScopeRegistry::new();
    let blank_scope = registry.intern("ui.linenr");
    let scope = registry.intern("diagnostic");
    let empty_lane = SignColumn::with_width(0, blank_scope); // no sources — width collapsed
    let mut content_lane = SignColumn::with_width(2, blank_scope);
    content_lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "!".into(),
            scope,
            priority: 1,
        },
    }));

    let graphemes = vec![crate::types::Grapheme {
        byte_range: 0..1,
        char_offset: 0,
        display_col: 0,
        width: 1,
        content: crate::types::CellContent::Grapheme,
        indent_depth: 0,
        scope: None,
    }];
    let rows = [crate::types::DisplayRow {
        kind: RowKind::LineStart { line_idx: 0 },
        graphemes: 0..1,
    }];
    let styles = vec![crate::types::ResolvedStyle::default()];
    let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> =
        vec![(0, Box::new(empty_lane)), (1, Box::new(content_lane))];
    let visible = crate::layout::PaneGeometry {
        content_height: 1,
        content_width: 6,
        gutter_width: 2, // 0 (empty_lane) + 2 (content_lane)
        last_line_idx: 0,
    };
    let viewport = crate::pane::ViewportState::new(8, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 1,
    };
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    let mut theme = Theme::default();
    theme.bake(&registry);
    let rope = ropey::Rope::from_str("x\n");
    let lane_widths = vec![0u16, 2u16];
    let compose_ctx = crate::render::ComposeCtx {
        gutter_columns: &gutter_columns,
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        rope: &rope,
        default_gutter_scope: blank_scope,
    };
    let mut canvas = crate::render::Canvas::new(&mut buf, &theme, None);
    crate::render::compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "x",
        "",
        0,
        &lane_widths,
        &compose_ctx,
        &mut canvas,
        None,
    );

    let sym = |x: u16| {
        buf.cell(ratatui::layout::Position { x, y: 0 })
            .unwrap()
            .symbol()
            .to_string()
    };
    assert_eq!(
        sym(0),
        "!",
        "the width-2 column's sign starts right at x=0, unaffected by the width-0 column ahead of it"
    );
    assert_eq!(sym(1), " ", "separator cell");
    assert_eq!(sym(2), "x", "content starts exactly at gutter_width=2");
}

#[test]
fn sign_scope_resolves_via_baked_theme() {
    let mut registry = ScopeRegistry::new();
    let scope_id = registry.intern("diagnostic.error");
    let blank_scope = registry.intern("ui.linenr");
    let mut styles_map = std::collections::HashMap::new();
    styles_map.insert(
        "diagnostic.error",
        crate::types::ResolvedStyle {
            fg: Some(ratatui::style::Color::Red),
            ..Default::default()
        },
    );
    let mut theme = Theme::new(styles_map, crate::types::ResolvedStyle::default());
    theme.bake(&registry);

    let mut lane = SignColumn::new(blank_scope);
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "!".into(),
            scope: scope_id,
            priority: 1,
        },
    }));
    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 0 }, &ctx(&rope))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        theme.resolve(cell.scope).fg,
        Some(ratatui::style::Color::Red)
    );
}

#[test]
fn multi_slot_column_keeps_top_n_signs_by_priority() {
    let mut registry = ScopeRegistry::new();
    let a = registry.intern("a");
    let b = registry.intern("b");
    let c = registry.intern("c");
    let blank_scope = registry.intern("ui.linenr");

    let mut lane = SignColumn::with_width(3, blank_scope); // 2 sign slots
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "!".into(),
            scope: a,
            priority: 10,
        },
    }));
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "+".into(),
            scope: b,
            priority: 5,
        },
    }));
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "~".into(),
            scope: c,
            priority: 1,
        },
    }));

    let rope = ropey::Rope::new();
    let cells = lane.render_row_cells(RowKind::LineStart { line_idx: 0 }, &ctx(&rope));
    assert_eq!(cells.len(), 2, "width-3 column = 2 sign slots");
    assert_eq!(cells[0].as_str(), "!", "priority 10 first");
    assert_eq!(cells[1].as_str(), "+", "priority 5 second");
}

#[test]
fn multi_slot_column_pads_with_blank_when_fewer_signs_than_slots() {
    let mut registry = ScopeRegistry::new();
    let a = registry.intern("a");
    let blank_scope = registry.intern("ui.linenr");

    let mut lane = SignColumn::with_width(3, blank_scope); // 2 sign slots
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "!".into(),
            scope: a,
            priority: 10,
        },
    }));

    let rope = ropey::Rope::new();
    let cells = lane.render_row_cells(RowKind::LineStart { line_idx: 0 }, &ctx(&rope));
    assert_eq!(cells.len(), 2, "still 2 cells — padded to slot count");
    assert_eq!(cells[0].as_str(), "!");
    assert_eq!(cells[1].as_str(), " ", "unused slot is blank");
}

#[test]
fn multi_slot_column_ties_go_to_later_registered_source() {
    let mut registry = ScopeRegistry::new();
    let a = registry.intern("a");
    let b = registry.intern("b");
    let blank_scope = registry.intern("ui.linenr");

    let mut lane = SignColumn::with_width(3, blank_scope); // 2 sign slots
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "A".into(),
            scope: a,
            priority: 10,
        },
    }));
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "B".into(),
            scope: b,
            priority: 10,
        },
    }));

    let rope = ropey::Rope::new();
    let cells = lane.render_row_cells(RowKind::LineStart { line_idx: 0 }, &ctx(&rope));
    assert_eq!(cells[0].as_str(), "B", "same priority — later source wins");
    assert_eq!(cells[1].as_str(), "A");
}

#[test]
fn width_one_column_keeps_no_signs() {
    let mut registry = ScopeRegistry::new();
    let a = registry.intern("a");
    let blank_scope = registry.intern("ui.linenr");

    let mut lane = SignColumn::with_width(1, blank_scope); // 0 sign slots
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "!".into(),
            scope: a,
            priority: 10,
        },
    }));

    let rope = ropey::Rope::new();
    let cells = lane.render_row_cells(RowKind::LineStart { line_idx: 0 }, &ctx(&rope));
    assert!(cells.is_empty(), "width-1 column has 0 sign slots");
}

/// Multi-slot sign columns must render through the full `compose_gutter`
/// path, not just `render_row_cells` in isolation. This test catches the
/// bug where the `usable_per_cell` formula was wrong, causing all signs
/// to be truncated to empty.
#[test]
fn multi_slot_column_renders_through_compose_gutter() {
    let mut registry = ScopeRegistry::new();
    let a = registry.intern("a");
    let b = registry.intern("b");
    let blank_scope = registry.intern("ui.linenr");

    let mut lane = SignColumn::with_width(3, blank_scope); // 2 sign slots
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "!".into(),
            scope: a,
            priority: 10,
        },
    }));
    lane.add_source(Box::new(FixedSign {
        line: 0,
        sign: Sign {
            text: "+".into(),
            scope: b,
            priority: 5,
        },
    }));

    let graphemes = vec![crate::types::Grapheme {
        byte_range: 0..1,
        char_offset: 0,
        display_col: 0,
        width: 1,
        content: crate::types::CellContent::Grapheme,
        indent_depth: 0,
        scope: None,
    }];
    let rows = [crate::types::DisplayRow {
        kind: RowKind::LineStart { line_idx: 0 },
        graphemes: 0..1,
    }];
    let styles = vec![crate::types::ResolvedStyle::default()];
    let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> = vec![(0, Box::new(lane))];
    let visible = crate::layout::PaneGeometry {
        content_height: 1,
        content_width: 5,
        gutter_width: 3,
        last_line_idx: 0,
    };
    let viewport = crate::pane::ViewportState::new(8, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 1,
    };
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    let mut theme = Theme::default();
    theme.bake(&registry);
    let rope = ropey::Rope::from_str("x\n");
    let lane_widths = vec![3u16];
    let compose_ctx = crate::render::ComposeCtx {
        gutter_columns: &gutter_columns,
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        rope: &rope,
        default_gutter_scope: blank_scope,
    };
    let mut canvas = crate::render::Canvas::new(&mut buf, &theme, None);
    crate::render::compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "x",
        "",
        0,
        &lane_widths,
        &compose_ctx,
        &mut canvas,
        None,
    );

    let sym = |x: u16| {
        buf.cell(ratatui::layout::Position { x, y: 0 })
            .unwrap()
            .symbol()
            .to_string()
    };
    assert_eq!(sym(0), "!", "first sign slot renders");
    assert_eq!(sym(1), "+", "second sign slot renders");
    assert_eq!(sym(2), " ", "column right padding");
    assert_eq!(sym(3), "x", "content starts at gutter_width");
}
