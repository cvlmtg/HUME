use super::*;

#[test]
fn resolved_style_layer_fg_wins() {
    let base = ResolvedStyle {
        fg: Some(ratatui::style::Color::Red),
        ..Default::default()
    };
    let over = ResolvedStyle {
        fg: Some(ratatui::style::Color::Blue),
        ..Default::default()
    };
    assert_eq!(base.layer(over).fg, Some(ratatui::style::Color::Blue));
}

#[test]
fn resolved_style_layer_preserves_base_when_over_is_none() {
    let base = ResolvedStyle {
        fg: Some(ratatui::style::Color::Red),
        ..Default::default()
    };
    let over = ResolvedStyle::default();
    assert_eq!(base.layer(over).fg, Some(ratatui::style::Color::Red));
}

#[test]
fn resolved_style_layer_underline_none_preserves_base() {
    let base = ResolvedStyle {
        underline: UnderlineStyle::Wavy,
        ..Default::default()
    };
    let over = ResolvedStyle::default(); // underline = None
    assert_eq!(base.layer(over).underline, UnderlineStyle::Wavy);
}

#[test]
fn resolved_style_layer_underline_over_wins() {
    let base = ResolvedStyle {
        underline: UnderlineStyle::Wavy,
        ..Default::default()
    };
    let over = ResolvedStyle {
        underline: UnderlineStyle::Solid,
        ..Default::default()
    };
    assert_eq!(base.layer(over).underline, UnderlineStyle::Solid);
}

#[test]
fn resolved_style_layer_modifiers_empty_preserves_base() {
    let base = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..Default::default()
    };
    let over = ResolvedStyle::default();
    assert_eq!(base.layer(over).modifiers, Modifiers::BOLD);
}

#[test]
fn selection_range_ordered() {
    let sel = Selection {
        anchor: 42,
        head: 7,
    };
    let (start, end) = sel.range();
    assert!(start <= end);
    assert_eq!(start, 7);
    assert_eq!(end, 42);
}

#[test]
fn row_kind_line_idx() {
    assert_eq!(RowKind::LineStart { line_idx: 7 }.line_idx(), Some(7));
    assert_eq!(
        RowKind::Wrap {
            line_idx: 7,
            wrap_row: 1
        }
        .line_idx(),
        Some(7)
    );
    assert_eq!(
        RowKind::Virtual {
            provider_id: 0,
            anchor_line: 7
        }
        .line_idx(),
        None
    );
    assert_eq!(RowKind::Filler.line_idx(), None);
}

#[test]
fn resolved_style_layer_bg() {
    let base = ResolvedStyle {
        bg: Some(ratatui::style::Color::Red),
        ..Default::default()
    };
    let over = ResolvedStyle {
        bg: Some(ratatui::style::Color::Blue),
        ..Default::default()
    };
    assert_eq!(base.layer(over).bg, Some(ratatui::style::Color::Blue));
    // None over preserves base bg.
    assert_eq!(
        base.layer(ResolvedStyle::default()).bg,
        Some(ratatui::style::Color::Red)
    );
}

#[test]
fn resolved_style_layer_underline_color() {
    let base = ResolvedStyle {
        underline_color: Some(ratatui::style::Color::Green),
        ..Default::default()
    };
    let over = ResolvedStyle {
        underline_color: Some(ratatui::style::Color::Red),
        ..Default::default()
    };
    assert_eq!(
        base.layer(over).underline_color,
        Some(ratatui::style::Color::Red)
    );
    assert_eq!(
        base.layer(ResolvedStyle::default()).underline_color,
        Some(ratatui::style::Color::Green)
    );
}

#[test]
fn resolved_style_layer_modifiers_union() {
    let base = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..Default::default()
    };
    let over = ResolvedStyle {
        modifiers: Modifiers::ITALIC,
        ..Default::default()
    };
    assert_eq!(
        base.layer(over).modifiers,
        Modifiers::BOLD | Modifiers::ITALIC
    );
}

#[test]
fn resolved_style_to_ratatui_style() {
    let s = ResolvedStyle {
        fg: Some(ratatui::style::Color::Red),
        bg: Some(ratatui::style::Color::Blue),
        modifiers: Modifiers::BOLD | Modifiers::ITALIC | Modifiers::STRIKETHROUGH,
        underline: UnderlineStyle::Solid,
        underline_color: Some(ratatui::style::Color::Green),
    };
    let r: ratatui::style::Style = s.into();
    assert_eq!(r.fg, Some(ratatui::style::Color::Red));
    assert_eq!(r.bg, Some(ratatui::style::Color::Blue));
    assert!(r.add_modifier.contains(ratatui::style::Modifier::BOLD));
    assert!(r.add_modifier.contains(ratatui::style::Modifier::ITALIC));
    assert!(
        r.add_modifier
            .contains(ratatui::style::Modifier::CROSSED_OUT)
    );
    assert!(
        r.add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert_eq!(r.underline_color, Some(ratatui::style::Color::Green));
}

#[test]
fn new_modifiers_convert_to_ratatui() {
    let s = ResolvedStyle {
        modifiers: Modifiers::DIM
            | Modifiers::REVERSED
            | Modifiers::HIDDEN
            | Modifiers::SLOW_BLINK
            | Modifiers::RAPID_BLINK,
        ..Default::default()
    };
    let r: ratatui::style::Style = s.into();
    assert!(r.add_modifier.contains(ratatui::style::Modifier::DIM));
    assert!(r.add_modifier.contains(ratatui::style::Modifier::REVERSED));
    assert!(r.add_modifier.contains(ratatui::style::Modifier::HIDDEN));
    assert!(
        r.add_modifier
            .contains(ratatui::style::Modifier::SLOW_BLINK)
    );
    assert!(
        r.add_modifier
            .contains(ratatui::style::Modifier::RAPID_BLINK)
    );
}

#[test]
fn resolved_style_default_clears_all_modifiers() {
    // A style with nothing enabled must explicitly turn every modifier
    // OFF, so it can safely overwrite a cell that already carries stale
    // modifiers (e.g. a popup painted over syntax-highlighted text).
    let r: ratatui::style::Style = ResolvedStyle::default().into();
    assert_eq!(r.add_modifier, ratatui::style::Modifier::empty());
    assert_eq!(r.sub_modifier, ratatui::style::Modifier::all());
}

#[test]
fn resolved_style_sub_modifier_is_complement_of_add_modifier() {
    let s = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..Default::default()
    };
    let r: ratatui::style::Style = s.into();
    assert_eq!(r.add_modifier, ratatui::style::Modifier::BOLD);
    assert_eq!(
        r.sub_modifier,
        ratatui::style::Modifier::all() - ratatui::style::Modifier::BOLD
    );
    // No overlap between what's turned on and what's turned off.
    assert!((r.add_modifier & r.sub_modifier).is_empty());
}

#[test]
fn selection_range_anchor_equals_head() {
    let sel = Selection { anchor: 5, head: 5 };
    let (start, end) = sel.range();
    assert_eq!(start, 5);
    assert_eq!(end, 5);
}

#[test]
fn selection_is_collapsed() {
    assert!(Selection { anchor: 0, head: 0 }.is_collapsed());
    assert!(!Selection { anchor: 0, head: 1 }.is_collapsed());
}

#[test]
fn editor_mode_cursor_is_bar() {
    assert!(!EditorMode::Normal.cursor_is_bar());
    assert!(!EditorMode::Extend.cursor_is_bar());
    assert!(EditorMode::Insert.cursor_is_bar());
    assert!(EditorMode::Command.cursor_is_bar());
    assert!(EditorMode::Search.cursor_is_bar());
    assert!(EditorMode::Select.cursor_is_bar());
}
