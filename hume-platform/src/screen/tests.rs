use super::*;
use hume_grid::{Modifiers, Rgb, UnderlineStyle};

const RED: Rgb = Rgb(255, 0, 0);
const BLUE: Rgb = Rgb(0, 0, 255);

fn red() -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(RED),
        ..Default::default()
    }
}

/// The bytes one SGR update renders to — what the terminal actually receives
/// for a `from` → `to` style transition.
fn delta(from: ResolvedStyle, to: ResolvedStyle) -> String {
    Csi::Sgr(Sgr::Attributes(sgr_delta(&from, &to))).to_string()
}

fn painted(next: &Grid, prev: Option<&Grid>, cursor: Option<Position>) -> String {
    let mut out = String::new();
    frame(&mut out, next, prev, cursor);
    out
}

// ── Style deltas ─────────────────────────────────────────────────────────

#[test]
fn an_unchanged_style_needs_no_attributes() {
    // Load-bearing: termina renders empty attributes as a full SGR reset, so
    // the emitter must recognise "nothing changed" and write nothing at all.
    assert_eq!(sgr_delta(&red(), &red()), SgrAttributes::default());
}

#[test]
fn a_foreground_change_emits_true_colour() {
    assert_eq!(delta(ResolvedStyle::default(), red()), "\x1b[38;2;255;0;0m");
}

#[test]
fn a_cleared_foreground_emits_the_default_colour_code() {
    assert_eq!(delta(red(), ResolvedStyle::default()), "\x1b[39m");
}

#[test]
fn a_background_change_emits_true_colour() {
    let to = ResolvedStyle {
        bg: Some(BLUE),
        ..Default::default()
    };
    assert_eq!(delta(ResolvedStyle::default(), to), "\x1b[48;2;0;0;255m");
}

#[test]
fn a_cleared_background_emits_the_default_colour_code() {
    let from = ResolvedStyle {
        bg: Some(BLUE),
        ..Default::default()
    };
    assert_eq!(delta(from, ResolvedStyle::default()), "\x1b[49m");
}

#[test]
fn foreground_and_background_group_into_one_sequence() {
    let to = ResolvedStyle {
        fg: Some(RED),
        bg: Some(BLUE),
        ..Default::default()
    };
    assert_eq!(
        delta(ResolvedStyle::default(), to),
        "\x1b[38;2;255;0;0;48;2;0;0;255m"
    );
}

#[test]
fn underline_shapes_reach_the_wire() {
    // The whole reason this crate emits its own SGR: a backend that carries
    // only an on/off underline bit cannot express these at all.
    let shape = |u: UnderlineStyle| {
        delta(
            ResolvedStyle::default(),
            ResolvedStyle {
                underline: u,
                ..Default::default()
            },
        )
    };
    assert_eq!(shape(UnderlineStyle::Solid), "\x1b[4m");
    assert_eq!(shape(UnderlineStyle::Wavy), "\x1b[4:3m");
    assert_eq!(shape(UnderlineStyle::Dotted), "\x1b[4:4m");
    assert_eq!(shape(UnderlineStyle::Dashed), "\x1b[4:5m");
}

#[test]
fn a_removed_underline_is_turned_off() {
    let from = ResolvedStyle {
        underline: UnderlineStyle::Wavy,
        ..Default::default()
    };
    assert_eq!(delta(from, ResolvedStyle::default()), "\x1b[24m");
}

#[test]
fn an_underline_colour_emits_the_colon_form_and_its_own_reset() {
    let with = ResolvedStyle {
        underline: UnderlineStyle::Wavy,
        underline_color: Some(RED),
        ..Default::default()
    };
    let without = ResolvedStyle {
        underline: UnderlineStyle::Wavy,
        ..Default::default()
    };
    assert_eq!(delta(without, with), "\x1b[58:2::255:0:0m");
    assert_eq!(delta(with, without), "\x1b[59m");
}

#[test]
fn bold_is_set_by_clearing_intensity_first() {
    let bold = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..Default::default()
    };
    assert_eq!(delta(ResolvedStyle::default(), bold), "\x1b[22;1m");
}

#[test]
fn dropping_bold_while_keeping_dim_re_asserts_dim() {
    // SGR has one tri-state intensity, so turning bold off (22) also clears
    // dim. Deriving the update from added/removed bits alone emits a bare
    // 22 here and silently loses the dim the cell still asks for.
    let from = ResolvedStyle {
        modifiers: Modifiers::BOLD | Modifiers::DIM,
        ..Default::default()
    };
    let to = ResolvedStyle {
        modifiers: Modifiers::DIM,
        ..Default::default()
    };
    assert_eq!(delta(from, to), "\x1b[22;2m");
}

#[test]
fn dropping_all_intensity_emits_only_the_clear() {
    let from = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..Default::default()
    };
    assert_eq!(delta(from, ResolvedStyle::default()), "\x1b[22m");
}

#[test]
fn unchanged_intensity_emits_nothing_for_it() {
    let bold = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..Default::default()
    };
    let bold_italic = ResolvedStyle {
        modifiers: Modifiers::BOLD | Modifiers::ITALIC,
        ..Default::default()
    };
    assert_eq!(delta(bold, bold_italic), "\x1b[3m");
}

#[test]
fn dropping_slow_blink_while_keeping_rapid_re_asserts_rapid() {
    // Blink is the same tri-state shape as intensity.
    let from = ResolvedStyle {
        modifiers: Modifiers::SLOW_BLINK | Modifiers::RAPID_BLINK,
        ..Default::default()
    };
    let to = ResolvedStyle {
        modifiers: Modifiers::RAPID_BLINK,
        ..Default::default()
    };
    assert_eq!(delta(from, to), "\x1b[25;6m");
}

#[test]
fn boolean_modifiers_toggle_both_ways() {
    let pairs = [
        (Modifiers::ITALIC, "\x1b[3m", "\x1b[23m"),
        (Modifiers::REVERSED, "\x1b[7m", "\x1b[27m"),
        (Modifiers::HIDDEN, "\x1b[8m", "\x1b[28m"),
        (Modifiers::STRIKETHROUGH, "\x1b[9m", "\x1b[29m"),
    ];
    for (bit, on, off) in pairs {
        let set = ResolvedStyle {
            modifiers: bit,
            ..Default::default()
        };
        assert_eq!(delta(ResolvedStyle::default(), set), on, "{bit:?} on");
        assert_eq!(delta(set, ResolvedStyle::default()), off, "{bit:?} off");
    }
}

// ── Frame emission ───────────────────────────────────────────────────────

#[test]
fn an_unchanged_frame_writes_no_cells() {
    let g = Grid::new(4, 2);
    // Still brackets the frame: the cursor is hidden for the repaint and SGR
    // is left clean for whatever writes next.
    assert_eq!(painted(&g, Some(&g.clone()), None), "\x1b[?25l\x1b[m");
}

#[test]
fn one_changed_cell_is_positioned_styled_and_written() {
    let prev = Grid::new(3, 1);
    let mut next = prev.clone();
    next.set_glyph(1, 0, "a", 1, red());
    assert_eq!(
        painted(&next, Some(&prev), None),
        "\x1b[?25l\x1b[1;2H\x1b[38;2;255;0;0ma\x1b[m"
    );
}

#[test]
fn a_contiguous_run_is_positioned_once() {
    let prev = Grid::new(4, 1);
    let mut next = prev.clone();
    next.set_glyph(0, 0, "a", 1, red());
    next.set_glyph(1, 0, "b", 1, red());
    assert_eq!(
        painted(&next, Some(&prev), None),
        "\x1b[?25l\x1b[1;1H\x1b[38;2;255;0;0mab\x1b[m"
    );
}

#[test]
fn separate_runs_each_reposition_but_keep_the_running_style() {
    let prev = Grid::new(8, 1);
    let mut next = prev.clone();
    next.set_glyph(0, 0, "a", 1, red());
    next.set_glyph(6, 0, "b", 1, red());
    assert_eq!(
        painted(&next, Some(&prev), None),
        "\x1b[?25l\x1b[1;1H\x1b[38;2;255;0;0ma\x1b[1;7Hb\x1b[m"
    );
}

#[test]
fn a_wide_glyph_writes_once_and_its_continuation_writes_nothing() {
    let prev = Grid::new(4, 1);
    let mut next = prev.clone();
    next.set_glyph(0, 0, "コ", 2, red());
    assert_eq!(
        painted(&next, Some(&prev), None),
        "\x1b[?25l\x1b[1;1H\x1b[38;2;255;0;0mコ\x1b[m"
    );
}

#[test]
fn a_cell_after_a_wide_glyph_needs_no_reposition() {
    // The cursor advances by the glyph's own stored width, so the cell in
    // the column after a double-width glyph is already contiguous.
    let prev = Grid::new(4, 1);
    let mut next = prev.clone();
    next.set_glyph(0, 0, "コ", 2, red());
    next.set_glyph(2, 0, "a", 1, red());
    assert_eq!(
        painted(&next, Some(&prev), None),
        "\x1b[?25l\x1b[1;1H\x1b[38;2;255;0;0mコa\x1b[m"
    );
}

#[test]
fn rows_are_repositioned_separately() {
    let prev = Grid::new(3, 2);
    let mut next = prev.clone();
    next.set_glyph(0, 0, "a", 1, red());
    next.set_glyph(0, 1, "b", 1, red());
    assert_eq!(
        painted(&next, Some(&prev), None),
        "\x1b[?25l\x1b[1;1H\x1b[38;2;255;0;0ma\x1b[2;1Hb\x1b[m"
    );
}

#[test]
fn a_visible_cursor_is_placed_after_the_repaint() {
    let g = Grid::new(4, 1);
    assert_eq!(
        painted(&g, Some(&g.clone()), Some(Position::new(2, 0))),
        "\x1b[?25l\x1b[m\x1b[1;3H\x1b[?25h"
    );
}

#[test]
fn no_cursor_leaves_it_hidden() {
    let g = Grid::new(4, 1);
    assert!(!painted(&g, Some(&g.clone()), None).contains("\x1b[?25h"));
}

#[test]
fn a_full_redraw_emits_every_cell() {
    let mut g = Grid::new(3, 2);
    g.set_glyph(0, 0, "a", 1, red());
    assert_eq!(
        painted(&g, None, None),
        "\x1b[?25l\x1b[1;1H\x1b[38;2;255;0;0ma\x1b[39m  \x1b[2;1H   \x1b[m"
    );
}

// ── Screen resize ────────────────────────────────────────────────────────
//
// `Screen` itself needs a live `SharedTerm` to construct, so its resize
// decision is tested here as the pure function it delegates to instead.

#[test]
fn a_size_change_resizes_both_grids_and_forces_a_full_repaint() {
    let mut back = Grid::new(4, 2);
    let mut front = Grid::new(4, 2);
    let mut force_full = false;
    resize_if_needed(&mut back, &mut front, &mut force_full, 10, 3);
    assert_eq!(back.size(), (10, 3));
    assert_eq!(front.size(), (10, 3));
    assert!(force_full);
}

#[test]
fn an_unchanged_size_touches_neither_grid_nor_the_flag() {
    let mut back = Grid::new(4, 2);
    back.set_glyph(0, 0, "a", 1, red());
    let mut front = back.clone();
    let mut force_full = false;
    resize_if_needed(&mut back, &mut front, &mut force_full, 4, 2);
    // Content survives — only a size *change* discards it (see
    // `Grid::resize`'s own doc on why content is never preserved across one).
    assert_eq!(back.cell(0, 0).unwrap().text(), "a");
    assert!(!force_full);
}
