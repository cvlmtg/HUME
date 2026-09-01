use super::*;

// ── Default values match previous hardcoded constants ─────────────────────

#[test]
fn editor_settings_default_matches_old_constants() {
    let s = EditorSettings::default();
    assert_eq!(s.scrolloff, 3);
    assert_eq!(s.mouse_scroll_lines, 3);
    assert!(s.mouse_enabled);
    assert!(!s.mouse_select);
    assert_eq!(s.jump_list_capacity, 100);
    assert_eq!(s.jump_line_threshold, 5);
    assert_eq!(s.history_capacity, 100);
    assert_eq!(s.undo_levels, 0);
    assert_eq!(s.tab_width, 4);
    assert!(s.show_indent_guides);
    assert_eq!(s.tab_style, TabStyle::Hard);
    assert_eq!(s.wrap_mode, WrapMode::Indent { width: 0 });
    assert_eq!(s.line_number_style, LineNumberStyle::Hybrid);
    assert!(s.auto_pairs_enabled);
    assert!(s.select_changed_text);
    assert!(s.word_selects_whitespace);
    assert_eq!(s.word_chars, "");
    assert!(s.pane_dividers);
    assert!(s.statusline_mode_colors);
    assert_eq!(s.signcolumn, SignColumnConfig::default());
}

#[test]
fn buffer_overrides_default_is_all_none() {
    let ov = BufferOverrides::default();
    assert!(ov.tab_width.is_none());
    assert!(ov.show_indent_guides.is_none());
    assert!(ov.tab_style.is_none());
    assert!(ov.line_number_style.is_none());
    assert!(ov.auto_pairs_enabled.is_none());
    assert!(ov.select_changed_text.is_none());
    assert!(ov.word_selects_whitespace.is_none());
    assert!(ov.word_chars.is_none());
    assert!(ov.whitespace_space.is_none());
    assert!(ov.whitespace_tab.is_none());
    assert!(ov.whitespace_newline.is_none());
    assert!(ov.signcolumn.is_none());
}

// ── Resolution: override present → returns override value ─────────────────

#[test]
fn resolution_override_wins_over_global() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        tab_width: Some(8),
        ..Default::default()
    };
    assert_eq!(ov.tab_width(&global), 8);
}

#[test]
fn resolution_line_number_style_override_wins() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        line_number_style: Some(LineNumberStyle::Relative),
        ..Default::default()
    };
    assert_eq!(ov.line_number_style(&global), LineNumberStyle::Relative);
}

// ── Resolution: override absent → returns global value ────────────────────

#[test]
fn resolution_falls_back_to_global_tab_width() {
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    assert_eq!(ov.tab_width(&global), global.tab_width);
}

#[test]
fn resolution_tab_style_override_wins() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        tab_style: Some(TabStyle::Soft),
        ..Default::default()
    };
    assert_eq!(ov.tab_style(&global), TabStyle::Soft);
}

#[test]
fn resolution_falls_back_to_global_tab_style() {
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    assert_eq!(ov.tab_style(&global), global.tab_style);
}

// ── setting_value (get-option) ─────────────────────────────────────────

use hume_scripting::host::OptionValue;

#[test]
fn setting_value_bool_key_returns_bool() {
    let global = EditorSettings::default();
    assert_eq!(
        setting_value("mouse-enabled", &global, None),
        Some(OptionValue::Bool(true))
    );
}

#[test]
fn setting_value_usize_key_returns_int() {
    let global = EditorSettings::default();
    assert_eq!(
        setting_value("tab-width", &global, None),
        Some(OptionValue::Int(4))
    );
}

#[test]
fn setting_value_from_str_key_returns_str() {
    let global = EditorSettings::default();
    assert_eq!(
        setting_value("tab-style", &global, None),
        Some(OptionValue::Str("hard".to_string()))
    );
}

#[test]
fn setting_value_unknown_key_returns_none() {
    let global = EditorSettings::default();
    assert_eq!(setting_value("nonexistent", &global, None), None);
}

#[test]
fn setting_value_buffer_override_wins_over_global() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        tab_width: Some(8),
        ..Default::default()
    };
    assert_eq!(
        setting_value("tab-width", &global, Some(&ov)),
        Some(OptionValue::Int(8))
    );
}

#[test]
fn setting_value_falls_back_to_global_when_no_override() {
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    assert_eq!(
        setting_value("tab-width", &global, Some(&ov)),
        Some(OptionValue::Int(4))
    );
}

#[test]
fn setting_value_global_only_key_ignores_overrides_arg() {
    // "mouse-enabled" is global-only — passing `Some(&ov)` must not
    // change the outcome (there is no per-buffer storage for it).
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    assert_eq!(
        setting_value("mouse-enabled", &global, Some(&ov)),
        Some(OptionValue::Bool(true))
    );
}

#[test]
fn setting_value_subfield_key_falls_back_to_global_whitespace() {
    // Before the `subfield` macro section existed, `whitespace-space`/
    // `whitespace-tab`/`whitespace-newline` had no `setting_value` support
    // at all — `(get-option "whitespace-space")` returned "unknown setting".
    let mut global = EditorSettings::default();
    global.whitespace.space = WhitespaceRender::Trailing;
    global.whitespace.newline = true;
    assert_eq!(
        setting_value("whitespace-space", &global, None),
        Some(OptionValue::Str("trailing".to_string()))
    );
    assert_eq!(
        setting_value("whitespace-newline", &global, None),
        Some(OptionValue::Str("all".to_string()))
    );
}

#[test]
fn setting_value_whitespace_newline_round_trips_through_write_global() {
    // Independent-oracle guard, mirroring `setting_value_statusline_round_
    // trips_through_write_global`: write the wire string via write_global,
    // read it back via setting_value — must match, proving
    // format_show_newline really is parse_show_newline's inverse.
    for wire in SHOW_NEWLINE_VALUES {
        let mut s = EditorSettings::default();
        write_global("whitespace-newline", wire, &mut s).unwrap();
        assert_eq!(
            setting_value("whitespace-newline", &s, None),
            Some(OptionValue::Str(wire.to_string()))
        );
    }
}

#[test]
fn setting_value_subfield_key_buffer_override_wins_over_global() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        whitespace_tab: Some(WhitespaceRender::All),
        ..Default::default()
    };
    assert_eq!(
        setting_value("whitespace-tab", &global, Some(&ov)),
        Some(OptionValue::Str("all".to_string()))
    );
}

#[test]
fn setting_value_statusline_round_trips_through_write_global() {
    // Independent-oracle guard: write a wire string via write_global, then
    // read it back via setting_value — must match, proving format_statusline
    // really is parse_statusline's inverse. Before this, (get-option
    // "statusline") returned None ("unknown setting") even after a
    // successful :set global statusline=... write.
    let mut s = EditorSettings::default();
    write_global("statusline", "Mode,FileName||Position", &mut s).unwrap();
    assert_eq!(
        setting_value("statusline", &s, None),
        Some(OptionValue::Str("Mode,FileName||Position".to_string()))
    );
}

// ── TabStyle parsing ─────────────────────────────────────────────────────

#[test]
fn tab_style_parses_hard_soft_case_insensitive() {
    assert_eq!("hard".parse::<TabStyle>().unwrap(), TabStyle::Hard);
    assert_eq!("HARD".parse::<TabStyle>().unwrap(), TabStyle::Hard);
    assert_eq!("soft".parse::<TabStyle>().unwrap(), TabStyle::Soft);
    assert_eq!("SOFT".parse::<TabStyle>().unwrap(), TabStyle::Soft);
}

#[test]
fn tab_style_rejects_unknown() {
    assert!("bogus".parse::<TabStyle>().is_err());
}

#[test]
fn tab_style_values_round_trip_through_from_str() {
    // Independent-oracle guard: every completion-offered value must
    // actually parse, so `VALUES` can't silently drift from `FromStr`.
    for v in TabStyle::VALUES {
        assert!(
            v.parse::<TabStyle>().is_ok(),
            "'{v}' should parse as TabStyle"
        );
    }
}

// ── Auto-pairs resolution ─────────────────────────────────────────────────

#[test]
fn auto_pairs_ref_enabled_resolves_override_over_global() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        auto_pairs_enabled: Some(false),
        ..Default::default()
    };
    let (enabled, pairs) = ov.auto_pairs_ref(&global);
    assert!(!enabled);
    assert_eq!(pairs, hume_ops::auto_pairs::DEFAULT_PAIRS);
}

#[test]
fn auto_pairs_ref_enabled_falls_back_to_global_when_no_override() {
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    let (enabled, pairs) = ov.auto_pairs_ref(&global);
    assert_eq!(enabled, global.auto_pairs_enabled);
    assert_eq!(pairs, hume_ops::auto_pairs::DEFAULT_PAIRS);
}

// ── write_global ───────────────────────────────────────────────────────────

fn global(key: &str, value: &str) -> Result<EditorSettings, String> {
    let mut s = EditorSettings::default();
    write_global(key, value, &mut s)?;
    Ok(s)
}

fn buffer(key: &str, value: &str) -> Result<BufferOverrides, String> {
    let mut ov = BufferOverrides::default();
    write_buffer(key, value, &mut ov)?;
    Ok(ov)
}

#[test]
fn set_global_scrolloff() {
    assert_eq!(global("scrolloff", "1").unwrap().scrolloff, 1);
}

#[test]
fn set_global_pane_dividers() {
    assert!(!global("pane-dividers", "false").unwrap().pane_dividers);
}

#[test]
fn set_global_statusline_mode_colors() {
    assert!(
        !global("statusline.mode-colors", "false")
            .unwrap()
            .statusline_mode_colors
    );
}

#[test]
fn set_global_mouse_scroll_lines() {
    assert_eq!(
        global("mouse-scroll-lines", "5")
            .unwrap()
            .mouse_scroll_lines,
        5
    );
}

#[test]
fn set_global_mouse_enabled() {
    assert!(!global("mouse-enabled", "false").unwrap().mouse_enabled);
}

#[test]
fn set_global_mouse_select() {
    assert!(global("mouse-select", "true").unwrap().mouse_select);
}

#[test]
fn set_global_jump_list_capacity() {
    assert_eq!(
        global("jump-list-capacity", "50")
            .unwrap()
            .jump_list_capacity,
        50
    );
}

#[test]
fn set_global_jump_list_capacity_zero_errors() {
    assert!(global("jump-list-capacity", "0").is_err());
}

#[test]
fn set_global_jump_line_threshold() {
    assert_eq!(
        global("jump-line-threshold", "10")
            .unwrap()
            .jump_line_threshold,
        10
    );
}

#[test]
fn set_global_history_capacity() {
    assert_eq!(
        global("history-capacity", "50").unwrap().history_capacity,
        50
    );
}

#[test]
fn set_global_history_capacity_zero_errors() {
    assert!(global("history-capacity", "0").is_err());
}

#[test]
fn set_global_undo_levels() {
    assert_eq!(global("undo-levels", "30").unwrap().undo_levels, 30);
}

#[test]
fn set_global_undo_levels_zero_ok() {
    // Unlike history-capacity, 0 is the meaningful "unlimited" default here,
    // not a rejected value.
    assert_eq!(global("undo-levels", "0").unwrap().undo_levels, 0);
}

#[test]
fn set_global_tab_width() {
    assert_eq!(global("tab-width", "8").unwrap().tab_width, 8);
}

#[test]
fn set_global_tab_width_zero_errors() {
    assert!(global("tab-width", "0").is_err());
}

#[test]
fn set_global_lsp_viewport_debounce_ms() {
    assert_eq!(
        global("lsp.viewport-debounce-ms", "50")
            .unwrap()
            .lsp_viewport_debounce_ms,
        50
    );
}

#[test]
fn set_global_lsp_viewport_debounce_ms_zero_errors() {
    // 0 would fire on every frame during a scroll burst instead of
    // collapsing it into one OnViewportChange — defeats the setting.
    assert!(global("lsp.viewport-debounce-ms", "0").is_err());
}

#[test]
fn set_global_lsp_format_max_ranges() {
    assert_eq!(
        global("lsp.format-max-ranges", "32")
            .unwrap()
            .lsp_format_max_ranges,
        32
    );
}

#[test]
fn set_global_lsp_format_max_ranges_zero_errors() {
    // 0 would make every multi-range :lsp-fmt fall back to the
    // primary-selection-only path, defeating the setting's purpose.
    assert!(global("lsp.format-max-ranges", "0").is_err());
}

#[test]
fn set_global_tab_style() {
    assert_eq!(
        global("tab-style", "soft").unwrap().tab_style,
        TabStyle::Soft
    );
}

#[test]
fn set_global_tab_style_invalid_errors() {
    assert!(global("tab-style", "bogus").is_err());
}

#[test]
fn set_global_line_number_style() {
    assert_eq!(
        global("line-number-style", "relative")
            .unwrap()
            .line_number_style,
        LineNumberStyle::Relative,
    );
}

#[test]
fn set_global_wrap_mode_none() {
    assert_eq!(
        global("wrap-mode", "none").unwrap().wrap_mode,
        WrapMode::None
    );
}

#[test]
fn set_global_wrap_mode_indent() {
    assert_eq!(
        global("wrap-mode", "indent:80").unwrap().wrap_mode,
        WrapMode::Indent { width: 80 },
    );
}

#[test]
fn set_global_wrap_mode_indent_no_colon() {
    assert_eq!(
        global("wrap-mode", "indent").unwrap().wrap_mode,
        WrapMode::Indent { width: 0 },
    );
}

#[test]
fn set_global_wrap_mode_soft_no_colon() {
    assert_eq!(
        global("wrap-mode", "soft").unwrap().wrap_mode,
        WrapMode::Soft { width: 0 },
    );
}

#[test]
fn set_global_auto_pairs_enabled() {
    assert!(
        !global("auto-pairs-enabled", "false")
            .unwrap()
            .auto_pairs_enabled
    );
}

#[test]
fn set_global_indent_guides() {
    assert!(!global("indent-guides", "false").unwrap().show_indent_guides);
}

#[test]
fn set_global_select_changed_text() {
    assert!(
        !global("select-changed-text", "false")
            .unwrap()
            .select_changed_text
    );
}

#[test]
fn set_global_word_selects_whitespace() {
    assert!(
        !global("word-selects-whitespace", "false")
            .unwrap()
            .word_selects_whitespace
    );
}

#[test]
fn set_global_word_chars() {
    assert_eq!(global("word-chars", "-").unwrap().word_chars, "-");
}

#[test]
fn word_chars_rejects_whitespace() {
    // Validated at the `write_global`/`write_buffer` chokepoint — a value
    // containing whitespace or newline is refused, never stored.
    assert!(global("word-chars", "- ").is_err());
    assert!(buffer("word-chars", "-\n").is_err());
}

#[test]
fn set_global_whitespace_space() {
    assert_eq!(
        global("whitespace-space", "all").unwrap().whitespace.space,
        WhitespaceRender::All,
    );
}

#[test]
fn set_global_whitespace_tab() {
    assert_eq!(
        global("whitespace-tab", "trailing").unwrap().whitespace.tab,
        WhitespaceRender::Trailing,
    );
}

#[test]
fn set_global_whitespace_newline() {
    assert!(
        global("whitespace-newline", "all")
            .unwrap()
            .whitespace
            .newline,
    );
    assert!(
        !global("whitespace-newline", "none")
            .unwrap()
            .whitespace
            .newline,
    );
}

#[test]
fn set_global_whitespace_newline_trailing_rejected() {
    // `trailing` is meaningless for newlines (always at end-of-line) —
    // only `none`/`all` are accepted.
    let err = global("whitespace-newline", "trailing").err().unwrap();
    assert!(
        err.contains("none or all"),
        "expected 'none or all' in error: {err}"
    );
}

#[test]
fn show_newline_values_round_trip_through_parse_show_newline() {
    // Independent-oracle guard, mirroring `whitespace_render_values_
    // round_trip_through_from_str` (hume-engine/src/pane.rs): every
    // completion-offered value must actually parse, so `SHOW_NEWLINE_
    // VALUES` can't silently drift from `parse_show_newline`.
    for v in SHOW_NEWLINE_VALUES {
        let parsed = parse_show_newline(v);
        assert!(parsed.is_ok(), "'{v}' should parse via parse_show_newline");
        assert_eq!(
            format_show_newline(parsed.unwrap()),
            *v,
            "format_show_newline should be parse_show_newline's inverse for '{v}'"
        );
    }
}

#[test]
fn set_global_unknown_key_errors() {
    assert!(global("nonexistent", "42").is_err());
}

#[test]
fn set_global_invalid_value_errors() {
    assert!(global("scrolloff", "abc").is_err());
}

#[test]
fn set_global_empty_value_errors() {
    assert!(global("scrolloff", "").is_err());
    assert!(global("tab-width", "").is_err());
    assert!(global("mouse-enabled", "").is_err());
}

// ── write_buffer ───────────────────────────────────────────────────────────

#[test]
fn set_buffer_tab_width() {
    let global = EditorSettings::default();
    let ov = buffer("tab-width", "8").unwrap();
    assert_eq!(ov.tab_width(&global), 8);
}

#[test]
fn set_buffer_tab_style() {
    let global = EditorSettings::default();
    let ov = buffer("tab-style", "soft").unwrap();
    assert_eq!(ov.tab_style(&global), TabStyle::Soft);
}

#[test]
fn set_buffer_wrap_mode() {
    let global = EditorSettings::default();
    let ov = buffer("wrap-mode", "none").unwrap();
    assert_eq!(ov.wrap_mode(&global), WrapMode::None);
}

#[test]
fn set_buffer_line_number_style() {
    let global = EditorSettings::default();
    let ov = buffer("line-number-style", "absolute").unwrap();
    assert_eq!(
        ov.line_number_style(&global),
        hume_engine::builtins::line_number::LineNumberStyle::Absolute,
    );
}

#[test]
fn set_buffer_auto_pairs_enabled() {
    let global = EditorSettings::default();
    let ov = buffer("auto-pairs-enabled", "false").unwrap();
    let (enabled, _) = ov.auto_pairs_ref(&global);
    assert!(!enabled);
}

#[test]
fn set_buffer_select_changed_text() {
    let global = EditorSettings::default();
    let ov = buffer("select-changed-text", "false").unwrap();
    assert!(!ov.select_changed_text(&global));
}

#[test]
fn set_buffer_word_selects_whitespace() {
    let global = EditorSettings::default();
    let ov = buffer("word-selects-whitespace", "false").unwrap();
    assert!(!ov.word_selects_whitespace(&global));
}

#[test]
fn set_buffer_word_chars() {
    let global = EditorSettings::default();
    let ov = buffer("word-chars", "-").unwrap();
    assert_eq!(ov.word_chars(&global), "-");
}

#[test]
fn set_buffer_indent_guides() {
    let global = EditorSettings::default();
    let ov = buffer("indent-guides", "false").unwrap();
    assert!(!ov.show_indent_guides(&global));
}

#[test]
fn set_buffer_whitespace_space() {
    let global = EditorSettings::default();
    let ov = buffer("whitespace-space", "all").unwrap();
    assert_eq!(ov.whitespace(&global).space, WhitespaceRender::All);
}

#[test]
fn set_buffer_whitespace_tab() {
    let global = EditorSettings::default();
    let ov = buffer("whitespace-tab", "trailing").unwrap();
    assert_eq!(ov.whitespace(&global).tab, WhitespaceRender::Trailing);
}

#[test]
fn set_buffer_whitespace_newline() {
    let global = EditorSettings::default();
    let ov = buffer("whitespace-newline", "all").unwrap();
    assert!(ov.whitespace(&global).newline);
}

#[test]
fn set_buffer_whitespace_fields_are_independent() {
    // Overriding one sub-field leaves the others resolved from global,
    // even when the global has non-default values.
    let mut global = EditorSettings::default();
    global.whitespace.tab = WhitespaceRender::Trailing;
    let ov = buffer("whitespace-space", "all").unwrap();
    let ws = ov.whitespace(&global);
    assert_eq!(ws.space, WhitespaceRender::All); // from buffer override
    assert_eq!(ws.tab, WhitespaceRender::Trailing); // inherited from global
    assert!(!ws.newline); // inherited from global (default: off)
}

#[test]
fn set_buffer_global_only_setting_errors() {
    let mut ov = BufferOverrides::default();
    let err = write_buffer("scrolloff", "3", &mut ov).unwrap_err();
    assert!(
        err.contains("global-only"),
        "expected 'global-only' in error: {err}"
    );
}

#[test]
fn set_buffer_global_only_all_keys_error() {
    // Derived from `all_setting_keys()` filtered to exactly `[Scope::Global]`
    // (no Buffer, no Pane), rather than a hand-copied list — self-maintaining
    // against a newly added global-only key.
    let mut ov = BufferOverrides::default();
    for key in all_setting_keys() {
        if !matches!(setting_scopes(key), [Scope::Global]) {
            continue;
        }
        let err = write_buffer(key, "1", &mut ov).unwrap_err();
        assert!(
            err.contains("global-only"),
            "key '{key}': expected 'global-only' in error: {err}",
        );
    }
}

#[test]
fn set_buffer_unknown_key_errors() {
    assert!(buffer("nonexistent", "42").is_err());
}

#[test]
fn set_global_whitespace_invalid_value_errors() {
    assert!(global("whitespace-space", "bogus").is_err());
    assert!(global("whitespace-tab", "bogus").is_err());
    assert!(global("whitespace-newline", "bogus").is_err());
}

#[test]
fn set_buffer_whitespace_invalid_value_errors() {
    assert!(buffer("whitespace-space", "bogus").is_err());
    assert!(buffer("whitespace-tab", "bogus").is_err());
    assert!(buffer("whitespace-newline", "bogus").is_err());
}

#[test]
fn set_global_tab_width_propagates_to_unoverridden_buffer() {
    let mut global = EditorSettings::default();
    let ov = BufferOverrides::default();
    write_global("tab-width", "2", &mut global).unwrap();
    // Text has no override, so it inherits the new global value.
    assert_eq!(ov.tab_width(&global), 2);
}

#[test]
fn set_global_tab_style_propagates_to_unoverridden_buffer() {
    let mut global = EditorSettings::default();
    let ov = BufferOverrides::default();
    write_global("tab-style", "soft", &mut global).unwrap();
    assert_eq!(ov.tab_style(&global), TabStyle::Soft);
}

#[test]
fn apply_statusline_wrong_section_count_errors() {
    let mut s = EditorSettings::default();
    // Two pipes required; one pipe produces only two parts.
    assert!(write_global("statusline", "Mode|Position", &mut s).is_err());
    // Three pipes / four sections produce four parts, also rejected.
    assert!(write_global("statusline", "Mode|Position|Cwd|Extra", &mut s).is_err());
}

#[test]
fn apply_statusline_unknown_element_name_errors() {
    let mut s = EditorSettings::default();
    assert!(write_global("statusline", "NotAnElement||", &mut s).is_err());
}

#[test]
fn apply_statusline_text_scope_rejected() {
    let mut ov = BufferOverrides::default();
    assert!(write_buffer("statusline", "||", &mut ov).is_err());
}

#[test]
fn is_bool_setting_matches_every_bool_field() {
    for key in [
        "mouse-enabled",
        "mouse-select",
        "popup-border",
        "pane-dividers",
        "auto-pairs-enabled",
        "select-changed-text",
        "indent-guides",
    ] {
        assert!(is_bool_setting(key), "'{key}' should be a bool setting");
    }
    for key in [
        "tab-style",
        "wrap-mode",
        "scrolloff",
        "whitespace-newline",
        "unknown-key",
    ] {
        assert!(
            !is_bool_setting(key),
            "'{key}' should not be a bool setting"
        );
    }
}

// ── all_setting_keys / write_global / write_buffer cross-check ────────────
//
// `all_setting_keys()` and `setting_scopes()` are both generated from the
// same `$gkey`/`$bkey`/`$skey`/`$mkey` token stream, with a non-empty
// `scope: […]` required by the macro's own grammar (`+` repetition) — so
// every key in `all_setting_keys()` having a declared scope is a structural
// guarantee, not something a test can catch drifting. What a test *can*
// catch: a key declared in `global`/`buffer`/`subfield`/`manual_keys` that
// has no matching arm in `write_global`/`write_buffer` (a typo in the
// hand-written `manual_keys` arms, since the macro-generated arms can't
// drift from their own key list) — falling through to the `_ => "unknown
// setting"` catch-all. That's what this guardrail checks.

#[test]
fn all_setting_keys_are_recognized_by_apply_setting() {
    for key in all_setting_keys() {
        // A value no parser accepts: for most keys this is rejected as an
        // *invalid value*, not as an *unrecognized key* — either outcome
        // is fine here, we only guard against the "unknown setting"
        // catch-all, which would mean the key isn't wired into
        // `write_global`/`write_buffer` at all.
        //
        // Checked independently rather than dispatching on `.first()`: every
        // buffer-capable key declares `scope: [Scope::Global, Scope::Buffer]`
        // (Global always first), so a `.first()`-only dispatch would never
        // reach `write_buffer` at all.
        let scopes = setting_scopes(key);
        assert!(!scopes.is_empty(), "key '{key}' has no declared scope");
        if scopes.contains(&Scope::Global) {
            let mut s = EditorSettings::default();
            if let Err(err) = write_global(key, "\u{0}garbage\u{0}", &mut s) {
                assert!(
                    !err.contains("unknown setting"),
                    "key '{key}' (global) from all_setting_keys() is not recognized: {err}"
                );
            }
        }
        if scopes.contains(&Scope::Buffer) {
            let mut ov = BufferOverrides::default();
            if let Err(err) = write_buffer(key, "\u{0}garbage\u{0}", &mut ov) {
                assert!(
                    !err.contains("unknown setting"),
                    "key '{key}' (buffer) from all_setting_keys() is not recognized: {err}"
                );
            }
        }
    }
}

#[test]
fn has_declared_resync_matches_keys_with_derived_state() {
    // Independent cross-check of the `resync: true` declarations against
    // `editor::settings_ops::resync_derived_state`'s actual match arms — see
    // that function's doc for the debug_assert! this backs.
    for key in [
        "history-capacity",
        "undo-levels",
        "jump-list-capacity",
        "theme",
    ] {
        assert!(
            has_declared_resync(key),
            "'{key}' should declare resync: true"
        );
    }
    for key in ["scrolloff", "tab-width", "mouse-enabled", "unknown-key"] {
        assert!(
            !has_declared_resync(key),
            "'{key}' should not declare resync: true"
        );
    }
}

// ── Pane-scope chokepoint lint ───────────────────────────────────────────────

#[test]
fn every_pane_scoped_key_has_a_typed_set_arm() {
    // `typed_file::typed_set`'s `Scope::Pane` match arm only handles
    // "wrap-mode" by name and `unreachable!()`s on anything else declared
    // `scope: [.., Scope::Pane]` here. This test is the forcing function:
    // add a second pane-scoped key without adding its `typed_set` arm, and
    // this fails immediately instead of panicking a live editor at `:set`
    // time.
    //
    // Fail oracle: add `Scope::Pane` to any other macro entry's `scope:`
    // list (e.g. `tab-width`) without touching `typed_set` — this test must
    // fail naming that key.
    let pane_scoped: Vec<&str> = all_setting_keys()
        .iter()
        .copied()
        .filter(|k| setting_scopes(k).contains(&Scope::Pane))
        .collect();
    assert_eq!(
        pane_scoped,
        vec!["wrap-mode"],
        "typed_file::typed_set's Scope::Pane match only has an arm for \
         \"wrap-mode\" — a new pane-scoped key here needs a matching arm \
         added there too"
    );
}

/// Scope is a lattice, not a set of alternatives: a pane override is the
/// narrowest place a setting can be pinned, so anything settable there must
/// also be settable for a whole buffer and for the whole editor. Without the
/// buffer rung, a per-language `on-language-set` hook (`set-buffer-option!`)
/// can't reach a pane-scoped setting at all — which is exactly the gap
/// `wrap-mode` used to have (`scope: [Scope::Global, Scope::Pane]`, no
/// `Scope::Buffer`) before it grew a buffer rung.
///
/// Fail oracle: change any entry's `scope:` list to
/// `[Scope::Global, Scope::Pane]` (wrap-mode's own pre-buffer-scope shape) —
/// this test fails naming that key and its missing `buffer` rung.
#[test]
fn every_pane_scoped_key_is_also_buffer_and_global_scoped() {
    let missing: Vec<String> = all_setting_keys()
        .iter()
        .copied()
        .filter(|k| setting_scopes(k).contains(&Scope::Pane))
        .flat_map(|k| {
            let scopes = setting_scopes(k);
            [Scope::Buffer, Scope::Global]
                .into_iter()
                .filter(move |s| !scopes.contains(s))
                .map(move |s| format!("'{k}' is pane-scoped but has no {} scope", s.as_str()))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "every pane-scoped key must also be buffer- and global-scoped: {missing:?}"
    );
}

// ── SignColumnConfig parsing ──────────────────────────────────────────────

#[test]
fn signcolumn_default_is_always_auto_sized() {
    let cfg = SignColumnConfig::default();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.pinned_slots, None);
    assert_eq!(cfg.slots_for(0), 1, "at least 1 slot even with zero signs");
}

#[test]
fn signcolumn_auto_size_follows_registered_source_count() {
    let cfg = SignColumnConfig::default();
    assert_eq!(
        cfg.slots_for(10),
        10,
        "auto-size grows to however many sign sources are registered"
    );
}

#[test]
fn signcolumn_auto_size_caps_at_max_slots() {
    let cfg = SignColumnConfig::default();
    assert_eq!(
        cfg.slots_for(1000),
        SignColumnConfig::MAX_SLOTS,
        "auto-size never exceeds the type-domain cap, however many sources are registered"
    );
}

#[test]
fn signcolumn_parses_always() {
    let cfg: SignColumnConfig = "always".parse().unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.pinned_slots, None, "bare always auto-sizes");
}

#[test]
fn signcolumn_parses_auto() {
    let cfg: SignColumnConfig = "auto".parse().unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Auto);
    assert_eq!(cfg.pinned_slots, None, "bare auto auto-sizes");
}

#[test]
fn signcolumn_bare_and_explicit_one_are_distinct() {
    let bare: SignColumnConfig = "always".parse().unwrap();
    let explicit: SignColumnConfig = "always:1".parse().unwrap();
    assert_ne!(
        bare, explicit,
        "bare always auto-sizes; always:1 pins to exactly one slot"
    );
}

#[test]
fn signcolumn_parses_always_with_columns() {
    let cfg: SignColumnConfig = "always:3".parse().unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.pinned_slots, Some(3));
    assert_eq!(cfg.slots_for(0), 3, "pinned count ignores the ladder");
}

#[test]
fn signcolumn_parses_auto_with_columns() {
    let cfg: SignColumnConfig = "auto:2".parse().unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Auto);
    assert_eq!(cfg.pinned_slots, Some(2));
    assert_eq!(cfg.slots_for(0), 2, "pinned count ignores the ladder");
}

#[test]
fn signcolumn_rejects_zero_columns() {
    assert!("always:0".parse::<SignColumnConfig>().is_err());
    assert!("auto:0".parse::<SignColumnConfig>().is_err());
}

#[test]
fn signcolumn_rejects_columns_above_127() {
    assert!("always:128".parse::<SignColumnConfig>().is_err());
    assert!("auto:255".parse::<SignColumnConfig>().is_err());
}

#[test]
fn signcolumn_values_round_trip_through_from_str() {
    // Independent-oracle guard: every completion-offered value must
    // actually parse, so `VALUES` can't silently drift from `FromStr`
    // (mirrors `tab_style_values_round_trip_through_from_str`).
    for v in SignColumnConfig::VALUES {
        assert!(
            v.parse::<SignColumnConfig>().is_ok(),
            "'{v}' should parse as SignColumnConfig"
        );
    }
}

#[test]
fn signcolumn_rejects_invalid_mode() {
    assert!("bogus".parse::<SignColumnConfig>().is_err());
    assert!("bogus:1".parse::<SignColumnConfig>().is_err());
}

#[test]
fn signcolumn_rejects_non_numeric_columns() {
    assert!("always:abc".parse::<SignColumnConfig>().is_err());
}

#[test]
fn signcolumn_display_round_trips() {
    for input in ["always", "auto", "always:1", "auto:1", "always:3", "auto:2"] {
        let cfg: SignColumnConfig = input.parse().unwrap();
        assert_eq!(cfg.to_string(), input);
    }
}

#[test]
fn set_global_signcolumn() {
    let s = global("signcolumn", "auto:2").unwrap();
    assert_eq!(s.signcolumn.mode, SignColumnMode::Auto);
    assert_eq!(s.signcolumn.pinned_slots, Some(2));
}

#[test]
fn set_buffer_signcolumn() {
    let global = EditorSettings::default();
    let ov = buffer("signcolumn", "always:3").unwrap();
    let cfg = ov.signcolumn(&global);
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.pinned_slots, Some(3));
}
