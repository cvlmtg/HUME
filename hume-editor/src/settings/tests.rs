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
    assert_eq!(s.tab_width, 4);
    assert_eq!(s.tab_style, TabStyle::Hard);
    assert_eq!(s.wrap_mode, WrapMode::Indent { width: 0 });
    assert_eq!(s.line_number_style, LineNumberStyle::Hybrid);
    assert!(s.auto_pairs_enabled);
    assert!(s.select_changed_text);
    assert!(s.word_selects_whitespace);
    assert!(s.pane_dividers);
    assert_eq!(s.signcolumn, SignColumnConfig::default());
}

#[test]
fn buffer_overrides_default_is_all_none() {
    let ov = BufferOverrides::default();
    assert!(ov.tab_width.is_none());
    assert!(ov.tab_style.is_none());
    assert!(ov.line_number_style.is_none());
    assert!(ov.auto_pairs_enabled.is_none());
    assert!(ov.select_changed_text.is_none());
    assert!(ov.word_selects_whitespace.is_none());
    assert!(ov.auto_pairs.is_none());
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
fn auto_pairs_override_enabled_only() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        auto_pairs_enabled: Some(false),
        ..Default::default()
    };
    let (enabled, pairs) = ov.auto_pairs_ref(&global);
    assert!(!enabled);
    // Pairs list inherited from global.
    assert_eq!(pairs.len(), global.auto_pairs.len());
}

#[test]
fn auto_pairs_both_inherited_when_no_override() {
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    let (enabled, pairs) = ov.auto_pairs_ref(&global);
    assert_eq!(enabled, global.auto_pairs_enabled);
    assert_eq!(pairs.len(), global.auto_pairs.len());
}

// ── apply_setting: Global scope ───────────────────────────────────────────

fn global(key: &str, value: &str) -> Result<EditorSettings, String> {
    let mut s = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    apply_setting(SettingScope::Global, key, value, &mut s, &mut ov)?;
    Ok(s)
}

fn buffer(key: &str, value: &str) -> Result<BufferOverrides, String> {
    let mut s = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    apply_setting(SettingScope::Text, key, value, &mut s, &mut ov)?;
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
fn set_global_tab_width() {
    assert_eq!(global("tab-width", "8").unwrap().tab_width, 8);
}

#[test]
fn set_global_tab_width_zero_errors() {
    assert!(global("tab-width", "0").is_err());
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
        assert!(
            parse_show_newline(v).is_ok(),
            "'{v}' should parse via parse_show_newline"
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

// ── apply_setting: Text scope ───────────────────────────────────────────

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
fn set_buffer_wrap_mode_rejected_as_global_only() {
    // wrap-mode is global-only: it seeds new panes' `Pane::wrap_mode`, the
    // live per-pane SSOT — there is no buffer-scoped override anymore.
    assert!(buffer("wrap-mode", "none").is_err());
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
    let mut s = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    let err = apply_setting(SettingScope::Text, "scrolloff", "3", &mut s, &mut ov).unwrap_err();
    assert!(
        err.contains("global-only"),
        "expected 'global-only' in error: {err}"
    );
}

#[test]
fn set_buffer_global_only_all_keys_error() {
    let mut s = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    for key in [
        "scrolloff",
        "mouse-scroll-lines",
        "mouse-enabled",
        "mouse-select",
        "jump-list-capacity",
        "jump-line-threshold",
        "history-capacity",
        "popup-border",
        "pane-dividers",
    ] {
        let err = apply_setting(SettingScope::Text, key, "1", &mut s, &mut ov).unwrap_err();
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
    let mut ov = BufferOverrides::default();
    apply_setting(SettingScope::Global, "tab-width", "2", &mut global, &mut ov).unwrap();
    // Text has no override, so it inherits the new global value.
    assert_eq!(ov.tab_width(&global), 2);
}

#[test]
fn set_global_tab_style_propagates_to_unoverridden_buffer() {
    let mut global = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    apply_setting(
        SettingScope::Global,
        "tab-style",
        "soft",
        &mut global,
        &mut ov,
    )
    .unwrap();
    assert_eq!(ov.tab_style(&global), TabStyle::Soft);
}

#[test]
fn apply_statusline_wrong_section_count_errors() {
    let mut s = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    // Two pipes required; one pipe produces only two parts.
    assert!(
        apply_setting(
            SettingScope::Global,
            "statusline",
            "Mode|Position",
            &mut s,
            &mut ov
        )
        .is_err()
    );
    // Three pipes / four sections produce four parts, also rejected.
    assert!(
        apply_setting(
            SettingScope::Global,
            "statusline",
            "Mode|Position|Cwd|Extra",
            &mut s,
            &mut ov
        )
        .is_err()
    );
}

#[test]
fn apply_statusline_unknown_element_name_errors() {
    let mut s = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    assert!(
        apply_setting(
            SettingScope::Global,
            "statusline",
            "NotAnElement||",
            &mut s,
            &mut ov
        )
        .is_err()
    );
}

#[test]
fn apply_statusline_text_scope_rejected() {
    let mut s = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    assert!(apply_setting(SettingScope::Text, "statusline", "||", &mut s, &mut ov).is_err());
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
    ] {
        assert!(is_bool_setting(key), "'{key}' should be a bool setting");
    }
    for key in ["tab-style", "wrap-mode", "scrolloff", "unknown-key"] {
        assert!(
            !is_bool_setting(key),
            "'{key}' should not be a bool setting"
        );
    }
}

// ── all_setting_keys / apply_setting cross-check ─────────────────────────
//
// `all_setting_keys()` and `setting_scopes()` are both generated from the
// same `$gkey`/`$bkey`/`$mkey` token stream, with a non-empty `scope: […]`
// required by the macro's own grammar (`+` repetition) — so every key in
// `all_setting_keys()` having a declared scope is a structural guarantee,
// not something a test can catch drifting. What a test *can* catch: a key
// declared in `global`/`buffer`/`manual_keys` that has no matching arm in
// `apply_setting` (a typo in the hand-written `manual_keys` arms, since
// the macro-generated arms can't drift from their own key list) — falling
// through to `apply_setting`'s `_ => "unknown setting"` catch-all. That's
// what this guardrail checks.

#[test]
fn all_setting_keys_are_recognized_by_apply_setting() {
    for key in all_setting_keys() {
        let scope = match setting_scopes(key).first() {
            Some(&"global") => SettingScope::Global,
            Some(&"buffer") => SettingScope::Text,
            other => panic!("key '{key}' has no usable first scope: {other:?}"),
        };
        let mut s = EditorSettings::default();
        let mut ov = BufferOverrides::default();
        // A value no parser accepts: for most keys this is rejected as an
        // *invalid value*, not as an *unrecognized key* — either outcome
        // is fine here, we only guard against the "unknown setting"
        // catch-all, which would mean the key isn't wired into
        // `apply_setting` at all.
        if let Err(err) = apply_setting(scope, key, "\u{0}garbage\u{0}", &mut s, &mut ov) {
            assert!(
                !err.contains("unknown setting"),
                "key '{key}' from all_setting_keys() is not recognized by apply_setting: {err}"
            );
        }
    }
}

// ── SignColumnConfig parsing ──────────────────────────────────────────────

#[test]
fn signcolumn_default_is_always_1() {
    let cfg = SignColumnConfig::default();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.columns, 1);
    assert_eq!(cfg.width(), 2);
}

#[test]
fn signcolumn_parses_always() {
    let cfg: SignColumnConfig = "always".parse().unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.columns, 1);
}

#[test]
fn signcolumn_parses_auto() {
    let cfg: SignColumnConfig = "auto".parse().unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Auto);
    assert_eq!(cfg.columns, 1);
}

#[test]
fn signcolumn_parses_always_with_columns() {
    let cfg: SignColumnConfig = "always:3".parse().unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.columns, 3);
    assert_eq!(cfg.width(), 4);
}

#[test]
fn signcolumn_parses_auto_with_columns() {
    let cfg: SignColumnConfig = "auto:2".parse().unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Auto);
    assert_eq!(cfg.columns, 2);
    assert_eq!(cfg.width(), 3);
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
    for input in ["always:1", "auto:1", "always:3", "auto:2"] {
        let cfg: SignColumnConfig = input.parse().unwrap();
        assert_eq!(cfg.to_string(), input);
    }
}

#[test]
fn set_global_signcolumn() {
    let s = global("signcolumn", "auto:2").unwrap();
    assert_eq!(s.signcolumn.mode, SignColumnMode::Auto);
    assert_eq!(s.signcolumn.columns, 2);
}

#[test]
fn set_buffer_signcolumn() {
    let global = EditorSettings::default();
    let ov = buffer("signcolumn", "always:3").unwrap();
    let cfg = ov.signcolumn(&global);
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.columns, 3);
}
