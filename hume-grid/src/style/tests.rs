use super::*;

fn fg(c: Rgb) -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(c),
        ..Default::default()
    }
}

#[test]
fn layer_over_wins_for_fg() {
    let base = fg(Rgb(255, 0, 0));
    let over = fg(Rgb(0, 0, 255));
    assert_eq!(base.layer(over).fg, Some(Rgb(0, 0, 255)));
}

#[test]
fn layer_none_inherits_from_below() {
    let base = fg(Rgb(255, 0, 0));
    assert_eq!(
        base.layer(ResolvedStyle::default()).fg,
        Some(Rgb(255, 0, 0))
    );
}

#[test]
fn layer_unions_modifiers() {
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
fn layer_empty_modifiers_preserve_the_base() {
    let base = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..Default::default()
    };
    assert_eq!(
        base.layer(ResolvedStyle::default()).modifiers,
        Modifiers::BOLD
    );
}

#[test]
fn layer_resolves_bg_both_ways() {
    let base = ResolvedStyle {
        bg: Some(Rgb(255, 0, 0)),
        ..Default::default()
    };
    let over = ResolvedStyle {
        bg: Some(Rgb(0, 0, 255)),
        ..Default::default()
    };
    assert_eq!(base.layer(over).bg, Some(Rgb(0, 0, 255)));
    assert_eq!(
        base.layer(ResolvedStyle::default()).bg,
        Some(Rgb(255, 0, 0))
    );
}

#[test]
fn layer_resolves_underline_color_both_ways() {
    let base = ResolvedStyle {
        underline_color: Some(Rgb(0, 255, 0)),
        ..Default::default()
    };
    let over = ResolvedStyle {
        underline_color: Some(Rgb(255, 0, 0)),
        ..Default::default()
    };
    assert_eq!(base.layer(over).underline_color, Some(Rgb(255, 0, 0)));
    assert_eq!(
        base.layer(ResolvedStyle::default()).underline_color,
        Some(Rgb(0, 255, 0))
    );
}

#[test]
fn layer_keeps_underline_shape_from_below_when_over_has_none() {
    let base = ResolvedStyle {
        underline: UnderlineStyle::Wavy,
        ..Default::default()
    };
    assert_eq!(
        base.layer(ResolvedStyle::default()).underline,
        UnderlineStyle::Wavy
    );
}

#[test]
fn layer_overrides_underline_shape() {
    let base = ResolvedStyle {
        underline: UnderlineStyle::Wavy,
        ..Default::default()
    };
    let over = ResolvedStyle {
        underline: UnderlineStyle::Dotted,
        ..Default::default()
    };
    assert_eq!(base.layer(over).underline, UnderlineStyle::Dotted);
}

#[test]
fn normalized_drops_underline_color_with_no_underline() {
    let s = ResolvedStyle {
        underline: UnderlineStyle::None,
        underline_color: Some(Rgb(1, 2, 3)),
        ..Default::default()
    };
    assert_eq!(s.normalized().underline_color, None);
}

#[test]
fn normalized_keeps_underline_color_when_underlined() {
    let s = ResolvedStyle {
        underline: UnderlineStyle::Wavy,
        underline_color: Some(Rgb(1, 2, 3)),
        ..Default::default()
    };
    assert_eq!(s.normalized().underline_color, Some(Rgb(1, 2, 3)));
}

#[test]
fn normalized_makes_invisible_difference_compare_equal() {
    // The reason normalization exists: these two render identically, so they
    // must not read as a change the diff has to repaint.
    let painted = ResolvedStyle::default();
    let dormant = ResolvedStyle {
        underline_color: Some(Rgb(9, 9, 9)),
        ..Default::default()
    };
    assert_ne!(painted, dormant);
    assert_eq!(painted.normalized(), dormant.normalized());
}
