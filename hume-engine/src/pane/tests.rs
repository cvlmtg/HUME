use super::*;
use crate::types::Selection;

#[test]
fn viewport_state_defaults() {
    let vp = ViewportState::new(80, 24);
    assert_eq!(vp.top_line, 0);
    assert_eq!(vp.top_row_offset, 0);
    assert_eq!(vp.horizontal_offset, 0);
    assert_eq!(vp.width, 80);
    assert_eq!(vp.height, 24);
}

// ── WrapMode::FromStr ────────────────────────────────────────────────

#[test]
fn wrap_mode_from_str_none() {
    assert_eq!("none".parse::<WrapMode>().unwrap(), WrapMode::None);
    assert_eq!("NONE".parse::<WrapMode>().unwrap(), WrapMode::None);
}

#[test]
fn wrap_mode_from_str_variants() {
    assert_eq!(
        "soft:80".parse::<WrapMode>().unwrap(),
        WrapMode::Soft { width: 80 }
    );
    assert_eq!(
        "word:40".parse::<WrapMode>().unwrap(),
        WrapMode::Word { width: 40 }
    );
    assert_eq!(
        "indent:76".parse::<WrapMode>().unwrap(),
        WrapMode::Indent { width: 76 }
    );
}

#[test]
fn wrap_mode_from_str_bare_keywords() {
    // Bare keyword (no colon) → sentinel width 0 (terminal width).
    assert_eq!(
        "soft".parse::<WrapMode>().unwrap(),
        WrapMode::Soft { width: 0 }
    );
    assert_eq!(
        "word".parse::<WrapMode>().unwrap(),
        WrapMode::Word { width: 0 }
    );
    assert_eq!(
        "indent".parse::<WrapMode>().unwrap(),
        WrapMode::Indent { width: 0 }
    );
}

#[test]
fn wrap_mode_from_str_colon_zero_is_sentinel() {
    // `:0` is the same sentinel as bare keyword.
    assert_eq!(
        "soft:0".parse::<WrapMode>().unwrap(),
        WrapMode::Soft { width: 0 }
    );
}

#[test]
fn wrap_mode_from_str_case_insensitive() {
    assert_eq!(
        "Soft:80".parse::<WrapMode>().unwrap(),
        WrapMode::Soft { width: 80 }
    );
    assert_eq!(
        "INDENT:76".parse::<WrapMode>().unwrap(),
        WrapMode::Indent { width: 76 }
    );
}

#[test]
fn wrap_mode_from_str_error_unknown_kind() {
    assert!("hard:80".parse::<WrapMode>().is_err());
}

#[test]
fn wrap_mode_from_str_error_non_numeric_width() {
    assert!("soft:abc".parse::<WrapMode>().is_err());
}

#[test]
fn wrap_mode_values_round_trip_through_from_str() {
    // Independent-oracle guard: every completion-offered value must
    // actually parse, so `VALUES` can't silently drift from `FromStr`.
    // One-directional: this can't catch a variant added to `FromStr` but
    // left out of `VALUES` (it would just silently vanish from
    // completion) — `wrap_mode_from_str_bare_keywords` above is the
    // closest thing to a reverse check, but it's a second
    // hand-maintained list, not a derived one.
    for v in WrapMode::VALUES {
        assert!(
            v.parse::<WrapMode>().is_ok(),
            "'{v}' should parse as WrapMode"
        );
    }
}

#[test]
fn wrap_mode_display_round_trips_through_from_str() {
    for mode in [
        WrapMode::None,
        WrapMode::Soft { width: 0 },
        WrapMode::Soft { width: 80 },
        WrapMode::Word { width: 40 },
        WrapMode::Indent { width: 76 },
    ] {
        let rendered = mode.to_string();
        assert_eq!(rendered.parse::<WrapMode>().unwrap(), mode);
    }
}

// ── WhitespaceRender::FromStr ─────────────────────────────────────────

#[test]
fn whitespace_render_from_str_all_variants() {
    assert_eq!(
        "none".parse::<WhitespaceRender>().unwrap(),
        WhitespaceRender::None
    );
    assert_eq!(
        "all".parse::<WhitespaceRender>().unwrap(),
        WhitespaceRender::All
    );
    assert_eq!(
        "trailing".parse::<WhitespaceRender>().unwrap(),
        WhitespaceRender::Trailing
    );
}

#[test]
fn whitespace_render_from_str_case_insensitive() {
    assert_eq!(
        "None".parse::<WhitespaceRender>().unwrap(),
        WhitespaceRender::None
    );
    assert_eq!(
        "ALL".parse::<WhitespaceRender>().unwrap(),
        WhitespaceRender::All
    );
    assert_eq!(
        "Trailing".parse::<WhitespaceRender>().unwrap(),
        WhitespaceRender::Trailing
    );
}

#[test]
fn whitespace_render_from_str_error() {
    let err = "always".parse::<WhitespaceRender>().unwrap_err();
    assert!(err.contains("always"), "error should contain input: {err}");
}

#[test]
fn whitespace_render_values_round_trip_through_from_str() {
    // Independent-oracle guard: every completion-offered value must
    // actually parse, so `VALUES` can't silently drift from `FromStr`.
    // One-directional: this can't catch a variant added to `FromStr` but
    // left out of `VALUES` (it would just silently vanish from
    // completion) — `whitespace_render_from_str_all_variants` above is
    // the closest thing to a reverse check, but it's a second
    // hand-maintained list, not a derived one.
    for v in WhitespaceRender::VALUES {
        assert!(
            v.parse::<WhitespaceRender>().is_ok(),
            "'{v}' should parse as WhitespaceRender"
        );
    }
}

#[test]
fn whitespace_render_display_round_trips_through_from_str() {
    // `option_value!`'s `from_str` kind (settings.rs) renders this type via
    // `to_string()` for `(get-option "whitespace-space"|"whitespace-tab")` —
    // this must round-trip through the same `FromStr` used to write it.
    for variant in [
        WhitespaceRender::None,
        WhitespaceRender::All,
        WhitespaceRender::Trailing,
    ] {
        let rendered = variant.to_string();
        assert_eq!(rendered.parse::<WhitespaceRender>().unwrap(), variant);
    }
}

#[test]
fn wrap_mode_wrap_width() {
    assert_eq!(WrapMode::None.wrap_width(), None);
    assert_eq!(WrapMode::Soft { width: 80 }.wrap_width(), Some(80));
    assert_eq!(WrapMode::Word { width: 40 }.wrap_width(), Some(40));
    assert_eq!(WrapMode::Indent { width: 60 }.wrap_width(), Some(60));
}

#[test]
fn wrap_mode_resolve() {
    // Sentinel → concrete.
    assert_eq!(
        WrapMode::Soft { width: 0 }.resolve(80),
        WrapMode::Soft { width: 80 }
    );
    assert_eq!(
        WrapMode::Word { width: 0 }.resolve(80),
        WrapMode::Word { width: 80 }
    );
    assert_eq!(
        WrapMode::Indent { width: 0 }.resolve(80),
        WrapMode::Indent { width: 80 }
    );
    // Concrete and None pass through unchanged.
    assert_eq!(
        WrapMode::Soft { width: 40 }.resolve(80),
        WrapMode::Soft { width: 40 }
    );
    assert_eq!(WrapMode::None.resolve(80), WrapMode::None);
}

#[test]
fn wrap_mode_is_wrapping() {
    assert!(!WrapMode::None.is_wrapping());
    assert!(WrapMode::Soft { width: 80 }.is_wrapping());
    assert!(WrapMode::Word { width: 80 }.is_wrapping());
    assert!(WrapMode::Indent { width: 80 }.is_wrapping());
    // Sentinel (width: 0 = terminal width) must still report is_wrapping()
    // = true; it must not be conflated with WrapMode::None.
    assert!(WrapMode::Indent { width: 0 }.is_wrapping());
    assert!(WrapMode::Soft { width: 0 }.is_wrapping());
}

// ── Pane::new / wrap-mode override seeding ───────────────────────────────

#[test]
fn pane_new_has_no_wrap_override_and_nothing_to_restore() {
    // A fresh pane inherits the buffer/global setting (no pane-level pin)
    // and has never been toggled off, so there is no `:wrap` restore target
    // yet — see `hume-editor`'s `pane_state::toggle_focused_wrap`.
    let pane = Pane::new(BufferId::default());
    let wrap = pane.wrap();
    assert_eq!(wrap.mode, None);
    assert_eq!(wrap.saved, None);
}

// ── remember_scroll / recall_scroll ─────────────────────────────────────

/// A real (non-null) `BufferId` — `BufferId::default()` is slotmap's null
/// key, which `SecondaryMap::insert` silently no-ops on, so `saved_scrolls`
/// (a `SecondaryMap`) needs a minted key for `remember_scroll` to actually
/// persist anything.
fn fresh_buffer_id() -> BufferId {
    let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
    sm.insert(())
}

#[test]
fn recall_scroll_clamps_top_line_to_the_buffers_current_last_content_line() {
    let bid = fresh_buffer_id();
    let mut pane = Pane::new(bid);

    // Save a scroll position deep into a buffer that was, at the time, tall.
    pane.viewport.top_line = 100;
    pane.remember_scroll();

    // The pane moves elsewhere, then recalls the same buffer — which has
    // since shrunk to a last content line of 3 (e.g. edited by another pane
    // in the meantime).
    pane.viewport.top_line = 0;
    pane.recall_scroll(bid, 3);

    assert_eq!(pane.viewport.top_line, 3);
}

#[test]
fn recall_scroll_leaves_an_in_range_top_line_untouched() {
    let bid = fresh_buffer_id();
    let mut pane = Pane::new(bid);

    pane.viewport.top_line = 4;
    pane.remember_scroll();

    pane.viewport.top_line = 0;
    pane.recall_scroll(bid, 100);

    assert_eq!(pane.viewport.top_line, 4);
}

#[test]
fn whitespace_config_defaults() {
    let wc = WhitespaceConfig::default();
    assert_eq!(wc.space, WhitespaceRender::None);
    assert_eq!(wc.tab, WhitespaceRender::None);
    assert!(!wc.newline);
    assert_eq!(wc.space_char, "·");
    assert_eq!(wc.tab_char, "→");
    assert_eq!(wc.newline_char, "⏎");
    assert_eq!(wc.nbsp_char, "⍽");
}

fn make_pane_at_char(head_char: usize) -> Pane {
    Pane {
        selections: vec![Selection {
            anchor: head_char,
            head: head_char,
        }],
        ..Pane::new(crate::pipeline::BufferId::default())
    }
}

#[test]
fn primary_head_line_returns_head_line() {
    // "aaa\nbbb\nccc" — line 0 is chars 0..3, line 1 is chars 4..7, line 2 is chars 8..11.
    // Char 8 (start of line 2) should resolve to line 2.
    let rope = ropey::Rope::from_str("aaa\nbbb\nccc");
    let pane = make_pane_at_char(8); // first char of line 2
    assert_eq!(pane.primary_head_line(&rope), 2);
}

#[test]
fn primary_head_line_uses_primary_idx() {
    // Two selections; primary_idx points to the second one (on line 2).
    // "aaa\nbbb\nccc": char 0 = line 0, char 8 = line 2.
    let rope = ropey::Rope::from_str("aaa\nbbb\nccc");
    let mut pane = make_pane_at_char(0); // first selection on line 0
    pane.selections.push(Selection { anchor: 8, head: 8 }); // second on line 2
    pane.primary_idx = 1;
    assert_eq!(pane.primary_head_line(&rope), 2);
}
