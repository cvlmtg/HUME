use super::ops::write_global_for_test;
use super::*;
use hume_core::{ConfigOrigin, DiagSeverity, OptionType, OptionValue, StatuslineSections};
use hume_editing::text::LineEnding;
use hume_platform::path::AbsolutePath;

// ── Default values ──────────────────────────────────────────────────────────

#[test]
fn editor_settings_default_values() {
    let s = EditorSettings::default();
    assert_eq!(s.scroll_margin, 3);
    assert_eq!(s.mouse_scroll_lines, 3);
    assert!(s.mouse);
    assert!(!s.mouse_select);
    assert_eq!(s.jump_list_capacity, 100);
    assert_eq!(s.jump_list_min_lines, 5);
    assert_eq!(s.prompt_history_levels, 100);
    assert_eq!(s.undo_levels, 0);
    assert_eq!(s.indent_width, 4);
    assert!(s.indent_guides);
    assert_eq!(s.indent_style, TabStyle::Hard);
    assert_eq!(s.wrap_mode, WrapMode::Indent { width: 0 });
    assert_eq!(s.gutter_line_numbers, LineNumberStyle::Hybrid);
    assert!(s.insert_auto_pairs);
    assert!(s.insert_select_on_exit);
    assert!(!s.comment_prefer_line);
    assert_eq!(s.word_selects_whitespace, WordSelectsWhitespace::Before);
    assert_eq!(s.word_chars, "");
    assert!(s.pane_dividers);
    assert!(s.statusline_mode_colors);
    assert_eq!(s.tabline, TablineVisibility::Dynamic);
    assert_eq!(s.gutter_signs, SignColumnConfig::default());
}

#[test]
fn buffer_overrides_default_is_all_none() {
    let ov = BufferOverrides::default();
    assert!(ov.indent_width.is_none());
    assert!(ov.indent_guides.is_none());
    assert!(ov.indent_style.is_none());
    assert!(ov.gutter_line_numbers.is_none());
    assert!(ov.insert_auto_pairs.is_none());
    assert!(ov.insert_select_on_exit.is_none());
    assert!(ov.comment_prefer_line.is_none());
    assert!(ov.word_selects_whitespace.is_none());
    assert!(ov.word_chars.is_none());
    assert!(ov.whitespace_space.is_none());
    assert!(ov.whitespace_tab.is_none());
    assert!(ov.whitespace_newline.is_none());
    assert!(ov.gutter_signs.is_none());
}

// ── Resolution: override present → returns override value ─────────────────

#[test]
fn resolution_override_wins_over_global() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        indent_width: Some(8),
        ..Default::default()
    };
    assert_eq!(ov.indent_width(&global), 8);
}

#[test]
fn resolution_line_number_style_override_wins() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        gutter_line_numbers: Some(LineNumberStyle::Relative),
        ..Default::default()
    };
    assert_eq!(ov.gutter_line_numbers(&global), LineNumberStyle::Relative);
}

// ── Resolution: override absent → returns global value ────────────────────

#[test]
fn resolution_falls_back_to_global_tab_width() {
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    assert_eq!(ov.indent_width(&global), global.indent_width);
}

#[test]
fn resolution_tab_style_override_wins() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        indent_style: Some(TabStyle::Soft),
        ..Default::default()
    };
    assert_eq!(ov.indent_style(&global), TabStyle::Soft);
}

#[test]
fn resolution_falls_back_to_global_tab_style() {
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    assert_eq!(ov.indent_style(&global), global.indent_style);
}

// ── read_global / read_buffer (get-option, get-buffer-option) ──────────

/// `key`'s value as `get-option` reads it, or as `get-buffer-option` does
/// with `overrides`. `None` for a key with no value there.
fn read_key(
    key: &str,
    settings: &EditorSettings,
    overrides: Option<&BufferOverrides>,
) -> Option<OptionValue> {
    let setting = SettingKey::named(key)?;
    match overrides {
        None => setting
            .global()
            .map(|key| read_global(key, settings).value()),
        Some(overrides) => match setting.buffer()? {
            BufferTarget::Override(key) => Some(read_buffer(key, settings, overrides).value()),
            BufferTarget::Intrinsic(_) => None,
        },
    }
}

#[test]
fn read_key_bool_key_returns_bool() {
    let global = EditorSettings::default();
    assert_eq!(
        read_key("mouse", &global, None),
        Some(OptionValue::Bool(true))
    );
}

#[test]
fn read_key_usize_key_returns_int() {
    let global = EditorSettings::default();
    assert_eq!(
        read_key("indent-width", &global, None),
        Some(OptionValue::Int(4))
    );
}

#[test]
fn read_key_enum_key_returns_its_name() {
    let global = EditorSettings::default();
    assert_eq!(
        read_key("indent-style", &global, None),
        Some(OptionValue::Str("hard".to_string()))
    );
}

/// A `name:N` value reads back bare for the content width, and with its
/// count otherwise.
#[test]
fn read_key_parameterized_keys_return_their_text() {
    let mut global = EditorSettings::default();
    assert_eq!(
        read_key("wrap-mode", &global, None),
        Some(OptionValue::Str("indent".to_string()))
    );
    global.wrap_mode = WrapMode::Word { width: 80 };
    assert_eq!(
        read_key("wrap-mode", &global, None),
        Some(OptionValue::Str("word:80".to_string()))
    );
    assert_eq!(
        read_key("gutter-signs", &global, None),
        Some(OptionValue::Str("always".to_string()))
    );
}

#[test]
fn read_key_shell_defaults_to_sh_c() {
    let global = EditorSettings::default();
    assert_eq!(
        read_key("shell", &global, None),
        Some(OptionValue::List(vec!["sh".to_string(), "-c".to_string()]))
    );
}

fn text(value: &str) -> OptionInput {
    OptionInput::Text(value.to_string())
}

fn words(items: &[&str]) -> OptionInput {
    OptionInput::Value(OptionValue::List(
        items.iter().map(|w| w.to_string()).collect(),
    ))
}

/// [`write_buffer`] for the setting a raw `key` names.
fn write_buffer_named(
    key: &str,
    value: &OptionInput,
    overrides: &mut BufferOverrides,
    by: ConfigOrigin,
) -> Result<bool, String> {
    let Some(BufferTarget::Override(key)) = SettingKey::named(key).and_then(SettingKey::buffer)
    else {
        return Err(format!("'{key}' has no buffer override"));
    };
    write_buffer(key, value, overrides, by)
}

#[test]
fn shell_reads_back_the_words_it_was_set_to() {
    for value in [text(r#""/opt/my sh" -lc"#), words(&["/opt/my sh", "-lc"])] {
        let mut s = EditorSettings::default();
        write_global_for_test("shell", &value, &mut s, ConfigOrigin::User).unwrap();
        assert_eq!(
            read_key("shell", &s, None),
            Some(OptionValue::List(vec![
                "/opt/my sh".to_string(),
                "-lc".to_string()
            ])),
            "{value:?}"
        );
    }
}

#[test]
fn shell_refuses_an_empty_program() {
    for value in [text("  "), text(r#""" -c"#), words(&["", "-c"]), words(&[])] {
        let mut s = EditorSettings::default();
        let err = write_global_for_test("shell", &value, &mut s, ConfigOrigin::User)
            .expect_err(&format!("{value:?} names no program"));
        assert!(err.starts_with("invalid value for 'shell'"), "{err}");
        assert_eq!(s.shell, ShellCommand::default());
    }
}

#[test]
fn only_the_shell_setting_takes_a_list() {
    let mut s = EditorSettings::default();
    let err = write_global_for_test(
        "word-chars",
        &words(&["_", "-"]),
        &mut s,
        ConfigOrigin::User,
    )
    .expect_err("word-chars is not a word list");
    assert!(err.starts_with("invalid value for 'word-chars'"), "{err}");
    let mut ov = BufferOverrides::default();
    assert!(
        write_buffer_named("indent-width", &words(&["4"]), &mut ov, ConfigOrigin::User).is_err()
    );
}

#[test]
fn read_key_buffer_override_wins_over_global() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        indent_width: Some(8),
        ..Default::default()
    };
    assert_eq!(
        read_key("indent-width", &global, Some(&ov)),
        Some(OptionValue::Int(8))
    );
}

#[test]
fn read_key_falls_back_to_global_when_no_override() {
    let global = EditorSettings::default();
    let ov = BufferOverrides::default();
    assert_eq!(
        read_key("indent-width", &global, Some(&ov)),
        Some(OptionValue::Int(4))
    );
}

#[test]
fn read_key_subfield_key_falls_back_to_global_whitespace() {
    let mut global = EditorSettings::default();
    global.whitespace.space = WhitespaceRender::Trailing;
    global.whitespace.newline = true;
    assert_eq!(
        read_key("whitespace-space", &global, None),
        Some(OptionValue::Str("trailing".to_string()))
    );
    assert_eq!(
        read_key("whitespace-newline", &global, None),
        Some(OptionValue::Str("all".to_string()))
    );
}

#[test]
fn read_key_whitespace_newline_round_trips_through_write_global() {
    // Write the wire string, read it back: they must match.
    for wire in <kind::ShowNewline as kind::Kind<bool>>::NAMES {
        let mut s = EditorSettings::default();
        write_global_for_test(
            "whitespace-newline",
            &text(wire),
            &mut s,
            ConfigOrigin::User,
        )
        .unwrap();
        assert_eq!(
            read_key("whitespace-newline", &s, None),
            Some(OptionValue::Str(wire.to_string()))
        );
    }
}

#[test]
fn read_key_subfield_key_buffer_override_wins_over_global() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        whitespace_tab: Some(WhitespaceRender::All),
        ..Default::default()
    };
    assert_eq!(
        read_key("whitespace-tab", &global, Some(&ov)),
        Some(OptionValue::Str("all".to_string()))
    );
}

#[test]
fn read_key_statusline_reads_back_the_sections_of_a_text_write() {
    let mut s = EditorSettings::default();
    write_global_for_test(
        "statusline",
        &text("Mode,FileName||Position"),
        &mut s,
        ConfigOrigin::User,
    )
    .unwrap();
    assert_eq!(
        read_key("statusline", &s, None),
        Some(OptionValue::Sections(StatuslineSections {
            left: vec!["Mode".to_string(), "FileName".to_string()],
            center: vec![],
            right: vec!["Position".to_string()],
        }))
    );
}

// ── TabStyle parsing ─────────────────────────────────────────────────────

#[test]
fn tab_style_parses_hard_soft_case_insensitive() {
    assert_eq!(TabStyle::parse("hard").unwrap(), TabStyle::Hard);
    assert_eq!(TabStyle::parse("HARD").unwrap(), TabStyle::Hard);
    assert_eq!(TabStyle::parse("soft").unwrap(), TabStyle::Soft);
    assert_eq!(TabStyle::parse("SOFT").unwrap(), TabStyle::Soft);
}

#[test]
fn tab_style_rejects_unknown() {
    assert!(TabStyle::parse("bogus").is_err());
}

#[test]
fn tab_style_values_round_trip_through_parse() {
    // Every completion-offered value must
    // actually parse, so `VALUES` can't silently drift from `parse`.
    for v in TabStyle::VALUES {
        assert!(TabStyle::parse(v).is_ok(), "'{v}' should parse as TabStyle");
    }
}

// ── CursorShape parsing ──────────────────────────────────────────────────

#[test]
fn cursor_shape_parses_case_insensitive() {
    assert_eq!(CursorShape::parse("block").unwrap(), CursorShape::Block);
    assert_eq!(CursorShape::parse("BLOCK").unwrap(), CursorShape::Block);
    assert_eq!(CursorShape::parse("bar").unwrap(), CursorShape::Bar);
    assert_eq!(
        CursorShape::parse("underline").unwrap(),
        CursorShape::Underline
    );
}

#[test]
fn cursor_shape_rejects_unknown() {
    assert!(CursorShape::parse("bogus").is_err());
}

#[test]
fn cursor_shape_values_round_trip_through_from_str() {
    for v in CursorShape::VALUES {
        assert!(
            CursorShape::parse(v).is_ok(),
            "'{v}' should parse as CursorShape"
        );
    }
}

// ── Auto-pairs resolution ─────────────────────────────────────────────────

#[test]
fn auto_pairs_ref_enabled_resolves_override_over_global() {
    let global = EditorSettings::default();
    let ov = BufferOverrides {
        insert_auto_pairs: Some(false),
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
    assert_eq!(enabled, global.insert_auto_pairs);
    assert_eq!(pairs, hume_ops::auto_pairs::DEFAULT_PAIRS);
}

// ── Origin ranking ─────────────────────────────────────────────────────────

#[test]
fn a_global_value_ranks_by_its_writers_origin() {
    let mut s = EditorSettings::default();
    let mut write =
        |value: &str, by| write_global_for_test("indent-width", &text(value), &mut s, by);
    assert_eq!(write("2", ConfigOrigin::User), Ok(true));
    assert_eq!(write("8", ConfigOrigin::Plugin), Ok(false));
    assert_eq!(write("3", ConfigOrigin::User), Ok(true));
    assert_eq!(s.indent_width, 3);
}

#[test]
fn a_plugins_global_value_gives_way_to_a_later_plugins_and_a_users() {
    let mut s = EditorSettings::default();
    let mut write =
        |value: &str, by| write_global_for_test("indent-width", &text(value), &mut s, by);
    assert_eq!(write("2", ConfigOrigin::Plugin), Ok(true));
    assert_eq!(write("5", ConfigOrigin::Plugin), Ok(true));
    assert_eq!(write("7", ConfigOrigin::User), Ok(true));
    assert_eq!(s.indent_width, 7);
}

#[test]
fn a_shadowed_global_write_still_rejects_a_bad_value() {
    let mut s = EditorSettings::default();
    write_global_for_test("indent-width", &text("2"), &mut s, ConfigOrigin::User).unwrap();
    assert!(
        write_global_for_test("indent-width", &text("wide"), &mut s, ConfigOrigin::Plugin).is_err()
    );
}

#[test]
fn global_origins_are_per_key() {
    let mut s = EditorSettings::default();
    write_global_for_test("indent-width", &text("2"), &mut s, ConfigOrigin::User).unwrap();
    assert_eq!(
        write_global_for_test("scroll-margin", &text("1"), &mut s, ConfigOrigin::Plugin),
        Ok(true)
    );
    assert_eq!(s.scroll_margin, 1);
}

#[test]
fn a_buffer_override_ranks_by_its_writers_origin() {
    let mut ov = BufferOverrides::default();
    let mut write = |value: &str, by| write_buffer_named("indent-width", &text(value), &mut ov, by);
    assert_eq!(write("2", ConfigOrigin::User), Ok(true));
    assert_eq!(write("8", ConfigOrigin::Plugin), Ok(false));
    assert_eq!(ov.indent_width, Some(2));
}

#[test]
fn default_settings_and_overrides_hold_no_origin() {
    let mut s = EditorSettings::default();
    let mut ov = BufferOverrides::default();
    assert_eq!(
        write_global_for_test("indent-width", &text("2"), &mut s, ConfigOrigin::Bundled),
        Ok(true)
    );
    assert_eq!(
        write_buffer_named("indent-width", &text("2"), &mut ov, ConfigOrigin::Bundled),
        Ok(true)
    );
}

// ── write_global ───────────────────────────────────────────────────────────

fn global(key: &str, value: &str) -> Result<EditorSettings, String> {
    let mut s = EditorSettings::default();
    write_global_for_test(key, &text(value), &mut s, ConfigOrigin::User)?;
    Ok(s)
}

fn buffer(key: &str, value: &str) -> Result<BufferOverrides, String> {
    let mut ov = BufferOverrides::default();
    write_buffer_named(key, &text(value), &mut ov, ConfigOrigin::User)?;
    Ok(ov)
}

#[test]
fn set_global_scroll_margin() {
    assert_eq!(global("scroll-margin", "1").unwrap().scroll_margin, 1);
}

#[test]
fn set_global_pane_dividers() {
    assert!(!global("pane-dividers", "false").unwrap().pane_dividers);
}

#[test]
fn set_global_statusline_mode_colors() {
    assert!(
        !global("statusline-mode-colors", "false")
            .unwrap()
            .statusline_mode_colors
    );
}

#[test]
fn set_global_tabline() {
    assert_eq!(
        global("tabline", "always").unwrap().tabline,
        TablineVisibility::Always
    );
    assert_eq!(
        global("tabline", "never").unwrap().tabline,
        TablineVisibility::Never
    );
    assert_eq!(
        global("tabline", "dynamic").unwrap().tabline,
        TablineVisibility::Dynamic
    );
}

#[test]
fn set_global_tabline_rejects_unknown_value() {
    assert!(global("tabline", "sometimes").is_err());
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
fn set_global_mouse() {
    assert!(!global("mouse", "false").unwrap().mouse);
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
        global("jump-list-min-lines", "10")
            .unwrap()
            .jump_list_min_lines,
        10
    );
}

#[test]
fn set_global_history_capacity() {
    assert_eq!(
        global("prompt-history-levels", "50")
            .unwrap()
            .prompt_history_levels,
        50
    );
}

#[test]
fn set_global_history_capacity_zero_errors() {
    assert!(global("prompt-history-levels", "0").is_err());
}

#[test]
fn set_global_undo_levels() {
    assert_eq!(global("undo-levels", "30").unwrap().undo_levels, 30);
}

#[test]
fn set_global_undo_levels_zero_ok() {
    // Unlike prompt-history-levels, 0 is the meaningful "unlimited" default here,
    // not a rejected value.
    assert_eq!(global("undo-levels", "0").unwrap().undo_levels, 0);
}

#[test]
fn set_global_tab_width() {
    assert_eq!(global("indent-width", "8").unwrap().indent_width, 8);
}

#[test]
fn set_global_tab_width_zero_errors() {
    assert!(global("indent-width", "0").is_err());
}

#[test]
fn set_global_viewport_debounce_ms() {
    assert_eq!(
        global("hook-viewport-debounce-ms", "50")
            .unwrap()
            .hook_viewport_debounce_ms,
        50
    );
}

#[test]
fn set_global_viewport_debounce_ms_zero_errors() {
    // 0 would fire on every frame during a scroll burst instead of
    // collapsing it into one OnViewportChange, which defeats the setting.
    assert!(global("hook-viewport-debounce-ms", "0").is_err());
}

#[test]
fn set_global_lsp_diagnostics_pull_debounce_ms() {
    assert_eq!(
        global("lsp-diagnostics-pull-debounce-ms", "50")
            .unwrap()
            .lsp_diagnostics_pull_debounce_ms,
        50
    );
}

#[test]
fn set_global_lsp_diagnostics_pull_debounce_ms_zero_errors() {
    assert!(global("lsp-diagnostics-pull-debounce-ms", "0").is_err());
}

#[test]
fn set_global_tab_style() {
    assert_eq!(
        global("indent-style", "soft").unwrap().indent_style,
        TabStyle::Soft
    );
}

#[test]
fn set_global_tab_style_invalid_errors() {
    assert!(global("indent-style", "bogus").is_err());
}

#[test]
fn set_global_cursor_shape_insert() {
    assert_eq!(
        global("insert-cursor-shape", "block")
            .unwrap()
            .insert_cursor_shape,
        CursorShape::Block
    );
}

#[test]
fn set_global_cursor_shape_insert_invalid_errors() {
    assert!(global("insert-cursor-shape", "bogus").is_err());
}

#[test]
fn set_global_line_number_style() {
    assert_eq!(
        global("gutter-line-numbers", "relative")
            .unwrap()
            .gutter_line_numbers,
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
fn set_global_auto_pairs() {
    assert!(
        !global("insert-auto-pairs", "false")
            .unwrap()
            .insert_auto_pairs
    );
}

#[test]
fn set_global_indent_guides() {
    assert!(!global("indent-guides", "false").unwrap().indent_guides);
}

#[test]
fn set_global_select_inserted_text() {
    assert!(
        !global("insert-select-on-exit", "false")
            .unwrap()
            .insert_select_on_exit
    );
}

#[test]
fn set_global_prefer_line_comments() {
    assert!(
        global("comment-prefer-line", "true")
            .unwrap()
            .comment_prefer_line
    );
}

#[test]
fn set_global_word_selects_whitespace() {
    for (text, value) in [
        ("off", WordSelectsWhitespace::Off),
        ("before", WordSelectsWhitespace::Before),
        ("after", WordSelectsWhitespace::After),
    ] {
        assert_eq!(
            global("word-selects-whitespace", text)
                .unwrap()
                .word_selects_whitespace,
            value
        );
    }
}

#[test]
fn set_global_word_selects_whitespace_rejects_bool() {
    assert!(global("word-selects-whitespace", "true").is_err());
    assert!(global("word-selects-whitespace", "false").is_err());
}

#[test]
fn set_global_word_chars() {
    assert_eq!(global("word-chars", "-").unwrap().word_chars, "-");
}

#[test]
fn word_chars_rejects_whitespace() {
    // Validated at the `write_global`/`write_buffer` chokepoint: a value
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
    // `trailing` is meaningless for newlines (always at end-of-line), so
    // only `none`/`all` are accepted.
    let err = global("whitespace-newline", "trailing").err().unwrap();
    assert!(
        err.contains("none or all"),
        "expected 'none or all' in error: {err}"
    );
}

#[test]
fn show_newline_values_round_trip_through_its_kind() {
    // Every completion-offered value must parse, and read back as itself.
    use kind::{Kind, ShowNewline};
    for v in <ShowNewline as Kind<bool>>::NAMES {
        let parsed = <ShowNewline as Kind<bool>>::parse_text(v)
            .unwrap_or_else(|e| panic!("'{v}' should parse: {e}"));
        assert_eq!(<ShowNewline as Kind<bool>>::shape(&parsed).text(), *v);
    }
}

#[test]
fn set_global_unknown_key_errors() {
    assert!(global("nonexistent", "42").is_err());
}

#[test]
fn set_global_invalid_value_errors() {
    assert!(global("scroll-margin", "abc").is_err());
}

#[test]
fn set_global_empty_value_errors() {
    assert!(global("scroll-margin", "").is_err());
    assert!(global("indent-width", "").is_err());
    assert!(global("mouse", "").is_err());
}

// ── write_buffer ───────────────────────────────────────────────────────────

#[test]
fn set_buffer_tab_width() {
    let global = EditorSettings::default();
    let ov = buffer("indent-width", "8").unwrap();
    assert_eq!(ov.indent_width(&global), 8);
}

#[test]
fn set_buffer_tab_style() {
    let global = EditorSettings::default();
    let ov = buffer("indent-style", "soft").unwrap();
    assert_eq!(ov.indent_style(&global), TabStyle::Soft);
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
    let ov = buffer("gutter-line-numbers", "absolute").unwrap();
    assert_eq!(
        ov.gutter_line_numbers(&global),
        hume_engine::builtins::line_number::LineNumberStyle::Absolute,
    );
}

#[test]
fn set_buffer_auto_pairs() {
    let global = EditorSettings::default();
    let ov = buffer("insert-auto-pairs", "false").unwrap();
    let (enabled, _) = ov.auto_pairs_ref(&global);
    assert!(!enabled);
}

#[test]
fn set_buffer_select_inserted_text() {
    let global = EditorSettings::default();
    let ov = buffer("insert-select-on-exit", "false").unwrap();
    assert!(!ov.insert_select_on_exit(&global));
}

#[test]
fn set_buffer_prefer_line_comments() {
    let global = EditorSettings::default();
    let ov = buffer("comment-prefer-line", "true").unwrap();
    assert!(ov.comment_prefer_line(&global));
}

#[test]
fn set_buffer_word_selects_whitespace() {
    let global = EditorSettings::default();
    let ov = buffer("word-selects-whitespace", "after").unwrap();
    assert_eq!(
        ov.word_selects_whitespace(&global),
        WordSelectsWhitespace::After
    );
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
    assert!(!ov.indent_guides(&global));
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
    write_global_for_test("indent-width", &text("2"), &mut global, ConfigOrigin::User).unwrap();
    // Text has no override, so it inherits the new global value.
    assert_eq!(ov.indent_width(&global), 2);
}

#[test]
fn set_global_tab_style_propagates_to_unoverridden_buffer() {
    let mut global = EditorSettings::default();
    let ov = BufferOverrides::default();
    write_global_for_test(
        "indent-style",
        &text("soft"),
        &mut global,
        ConfigOrigin::User,
    )
    .unwrap();
    assert_eq!(ov.indent_style(&global), TabStyle::Soft);
}

#[test]
fn apply_statusline_wrong_section_count_errors() {
    let mut s = EditorSettings::default();
    // Two pipes required; one pipe produces only two parts.
    assert!(
        write_global_for_test(
            "statusline",
            &text("Mode|Position"),
            &mut s,
            ConfigOrigin::User
        )
        .is_err()
    );
    // Three pipes / four sections produce four parts, also rejected.
    assert!(
        write_global_for_test(
            "statusline",
            &text("Mode|Position|Cwd|Extra"),
            &mut s,
            ConfigOrigin::User
        )
        .is_err()
    );
}

#[test]
fn apply_statusline_unknown_element_name_errors() {
    let mut s = EditorSettings::default();
    assert!(
        write_global_for_test(
            "statusline",
            &text("NotAnElement||"),
            &mut s,
            ConfigOrigin::User
        )
        .is_err()
    );
}

#[test]
fn apply_statusline_empty_element_name_errors() {
    let mut s = EditorSettings::default();
    let err = write_global_for_test(
        "statusline",
        &text("Mode,,FileName||"),
        &mut s,
        ConfigOrigin::User,
    )
    .unwrap_err();
    assert!(err.contains("empty element name"), "got: {err}");
    assert!(
        write_global_for_test("statusline", &text("Mode,||"), &mut s, ConfigOrigin::User).is_err(),
        "a trailing comma leaves an empty name too"
    );
}

#[test]
fn value_names_offer_true_and_false_for_every_bool_field() {
    let names = |key| {
        SettingKey::named(key)
            .and_then(SettingKey::global)
            .unwrap()
            .value_names()
    };
    for key in [
        "mouse",
        "mouse-select",
        "popup-border",
        "pane-dividers",
        "insert-auto-pairs",
        "insert-select-on-exit",
        "comment-prefer-line",
        "indent-guides",
    ] {
        assert_eq!(
            names(key),
            ["true", "false"],
            "'{key}' should be a bool setting"
        );
    }
    for key in [
        "indent-style",
        "wrap-mode",
        "scroll-margin",
        "whitespace-newline",
    ] {
        assert_ne!(
            names(key),
            ["true", "false"],
            "'{key}' should not be a bool setting"
        );
    }
}

// ── SignColumnConfig parsing ──────────────────────────────────────────────

#[test]
fn gutter_signs_default_is_always_auto_sized() {
    let cfg = SignColumnConfig::default();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.pinned_slots, None);
    assert_eq!(cfg.slots_for(0), 1, "at least 1 slot even with zero signs");
}

#[test]
fn gutter_signs_auto_size_follows_registered_source_count() {
    let cfg = SignColumnConfig::default();
    assert_eq!(
        cfg.slots_for(10),
        10,
        "auto-size grows to however many sign sources are registered"
    );
}

#[test]
fn gutter_signs_auto_size_caps_at_max_slots() {
    let cfg = SignColumnConfig::default();
    assert_eq!(
        cfg.slots_for(1000),
        SignColumnConfig::MAX_SLOTS,
        "auto-size never exceeds the type-domain cap, however many sources are registered"
    );
}

#[test]
fn gutter_signs_parses_always() {
    let cfg = SignColumnConfig::parse("always").unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.pinned_slots, None, "bare always auto-sizes");
}

#[test]
fn gutter_signs_parses_auto() {
    let cfg = SignColumnConfig::parse("auto").unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Auto);
    assert_eq!(cfg.pinned_slots, None, "bare auto auto-sizes");
}

#[test]
fn gutter_signs_bare_and_explicit_one_are_distinct() {
    let bare = SignColumnConfig::parse("always").unwrap();
    let explicit = SignColumnConfig::parse("always:1").unwrap();
    assert_ne!(
        bare, explicit,
        "bare always auto-sizes; always:1 pins to exactly one slot"
    );
}

#[test]
fn gutter_signs_parses_always_with_columns() {
    let cfg = SignColumnConfig::parse("always:3").unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.pinned_slots, Some(3));
    assert_eq!(cfg.slots_for(0), 3, "pinned count ignores the ladder");
}

#[test]
fn gutter_signs_parses_auto_with_columns() {
    let cfg = SignColumnConfig::parse("auto:2").unwrap();
    assert_eq!(cfg.mode, SignColumnMode::Auto);
    assert_eq!(cfg.pinned_slots, Some(2));
    assert_eq!(cfg.slots_for(0), 2, "pinned count ignores the ladder");
}

#[test]
fn gutter_signs_rejects_zero_columns() {
    assert!(SignColumnConfig::parse("always:0").is_err());
    assert!(SignColumnConfig::parse("auto:0").is_err());
}

#[test]
fn gutter_signs_rejects_columns_above_127() {
    assert!(SignColumnConfig::parse("always:128").is_err());
    assert!(SignColumnConfig::parse("auto:255").is_err());
}

#[test]
fn gutter_signs_values_round_trip_through_from_str() {
    // Every completion-offered value must
    // actually parse, so `VALUES` can't silently drift from `FromStr`
    // (mirrors `tab_style_values_round_trip_through_parse`).
    for v in SignColumnConfig::VALUES {
        assert!(
            SignColumnConfig::parse(v).is_ok(),
            "'{v}' should parse as SignColumnConfig"
        );
    }
}

#[test]
fn gutter_signs_rejects_invalid_mode() {
    assert!(SignColumnConfig::parse("bogus").is_err());
    assert!(SignColumnConfig::parse("bogus:1").is_err());
}

#[test]
fn gutter_signs_rejects_non_numeric_columns() {
    assert!(SignColumnConfig::parse("always:abc").is_err());
}

#[test]
fn gutter_signs_display_round_trips() {
    for input in ["always", "auto", "always:1", "auto:1", "always:3", "auto:2"] {
        let cfg = SignColumnConfig::parse(input).unwrap();
        assert_eq!(cfg.to_text(), input);
    }
}

#[test]
fn set_global_gutter_signs() {
    let s = global("gutter-signs", "auto:2").unwrap();
    assert_eq!(s.gutter_signs.mode, SignColumnMode::Auto);
    assert_eq!(s.gutter_signs.pinned_slots, Some(2));
}

#[test]
fn set_buffer_gutter_signs() {
    let global = EditorSettings::default();
    let ov = buffer("gutter-signs", "always:3").unwrap();
    let cfg = ov.gutter_signs(&global);
    assert_eq!(cfg.mode, SignColumnMode::Always);
    assert_eq!(cfg.pinned_slots, Some(3));
}

// `settings_enum!` generates the parse error from `VALUES`; these pin the
// exact wording, which is what a user sees after a typo in `init.scm`.
#[test]
fn settings_enum_parse_error_lists_every_accepted_value() {
    assert_eq!(
        ObjectJumpAlign::parse("bogus").unwrap_err(),
        "expected top, center, or off, got 'bogus'"
    );
    assert_eq!(
        CursorShape::parse("bogus").unwrap_err(),
        "expected block, bar, or underline, got 'bogus'"
    );
    assert_eq!(
        DiagSeverity::parse("bogus").unwrap_err(),
        "expected error, warning, info, or hint, got 'bogus'"
    );
}

#[test]
fn set_names_the_failing_key_in_enum_errors() {
    let mut settings = EditorSettings::default();
    assert_eq!(
        write_global_for_test(
            "whitespace-tab",
            &text("bogus"),
            &mut settings,
            ConfigOrigin::User
        )
        .unwrap_err(),
        "invalid value for 'whitespace-tab': expected none, all, or trailing, got 'bogus'"
    );
    assert_eq!(
        write_global_for_test(
            "whitespace-space",
            &text("bogus"),
            &mut settings,
            ConfigOrigin::User
        )
        .unwrap_err(),
        "invalid value for 'whitespace-space': expected none, all, or trailing, got 'bogus'"
    );
    assert_eq!(
        write_global_for_test(
            "wrap-mode",
            &text("sideways:3"),
            &mut settings,
            ConfigOrigin::User
        )
        .unwrap_err(),
        "invalid value for 'wrap-mode': expected none, soft[:N], word[:N], or indent[:N], got 'sideways:3'"
    );
}

#[test]
fn every_parser_kind_names_the_key_once() {
    let cases = [
        (
            "mouse",
            "maybe",
            "invalid value for 'mouse': expected true/false, got 'maybe'",
        ),
        (
            "scroll-margin",
            "x",
            "invalid value for 'scroll-margin': expected a non-negative integer, got 'x'",
        ),
        (
            "jump-list-capacity",
            "0",
            "invalid value for 'jump-list-capacity': must be at least 1",
        ),
        (
            "indent-width",
            "0",
            "invalid value for 'indent-width': must be at least 1",
        ),
        (
            "indent-width",
            "x",
            "invalid value for 'indent-width': expected 1–255, got 'x'",
        ),
        (
            "whitespace-newline",
            "trailing",
            "invalid value for 'whitespace-newline': expected none or all, got 'trailing'",
        ),
        (
            "word-chars",
            "- ",
            "invalid value for 'word-chars': cannot contain whitespace or newline: ' '",
        ),
        (
            "statusline",
            "Mode",
            "invalid value for 'statusline': expected three sections separated by '|' (e.g. 'Mode,FileName||Position'), got 'Mode'",
        ),
        (
            "statusline",
            "Mode,,x||",
            "invalid value for 'statusline': section 'Mode,,x' has an empty element name",
        ),
    ];
    for (key, value, expected) in cases {
        let mut settings = EditorSettings::default();
        assert_eq!(
            write_global_for_test(key, &text(value), &mut settings, ConfigOrigin::User)
                .unwrap_err(),
            expected,
            "{key}={value}"
        );
    }
}

#[test]
fn or_list_joins_one_two_and_many() {
    assert_eq!(super::or_list(&["a"]), "a");
    assert_eq!(super::or_list(&["a", "b"]), "a or b");
    assert_eq!(super::or_list(&["a", "b", "c"]), "a, b, or c");
}

// ── WrapMode text form ───────────────────────────────────────────────

#[test]
fn wrap_mode_parse_none() {
    assert_eq!(WrapMode::parse("none").unwrap(), WrapMode::None);
    assert_eq!(WrapMode::parse("NONE").unwrap(), WrapMode::None);
}

#[test]
fn wrap_mode_parse_variants() {
    assert_eq!(
        WrapMode::parse("soft:80").unwrap(),
        WrapMode::Soft { width: 80 }
    );
    assert_eq!(
        WrapMode::parse("word:40").unwrap(),
        WrapMode::Word { width: 40 }
    );
    assert_eq!(
        WrapMode::parse("indent:76").unwrap(),
        WrapMode::Indent { width: 76 }
    );
}

#[test]
fn wrap_mode_parse_bare_keywords() {
    // Bare keyword (no colon) → sentinel width 0 (content width).
    assert_eq!(
        WrapMode::parse("soft").unwrap(),
        WrapMode::Soft { width: 0 }
    );
    assert_eq!(
        WrapMode::parse("word").unwrap(),
        WrapMode::Word { width: 0 }
    );
    assert_eq!(
        WrapMode::parse("indent").unwrap(),
        WrapMode::Indent { width: 0 }
    );
}

#[test]
fn wrap_mode_parse_colon_zero_is_sentinel() {
    // `:0` is the same sentinel as bare keyword.
    assert_eq!(
        WrapMode::parse("soft:0").unwrap(),
        WrapMode::Soft { width: 0 }
    );
}

#[test]
fn wrap_mode_parse_case_insensitive() {
    assert_eq!(
        WrapMode::parse("Soft:80").unwrap(),
        WrapMode::Soft { width: 80 }
    );
    assert_eq!(
        WrapMode::parse("INDENT:76").unwrap(),
        WrapMode::Indent { width: 76 }
    );
}

#[test]
fn wrap_mode_parse_error_unknown_kind() {
    assert!(WrapMode::parse("hard:80").is_err());
}

#[test]
fn wrap_mode_parse_error_non_numeric_width() {
    assert!(WrapMode::parse("soft:abc").is_err());
}

#[test]
fn wrap_mode_values_round_trip_through_parse() {
    // Every completion-offered value must
    // actually parse, so `VALUES` can't silently drift from `parse`.
    // One-directional: this can't catch a variant added to `parse` but
    // left out of `VALUES` (it would just silently vanish from
    // completion). `wrap_mode_parse_bare_keywords` above is the
    // closest thing to a reverse check, but it's a second
    // hand-maintained list, not a derived one.
    for v in WrapMode::VALUES {
        assert!(WrapMode::parse(v).is_ok(), "'{v}' should parse as WrapMode");
    }
}

#[test]
fn wrap_mode_to_text_round_trips_through_parse() {
    for mode in [
        WrapMode::None,
        WrapMode::Soft { width: 0 },
        WrapMode::Soft { width: 80 },
        WrapMode::Word { width: 40 },
        WrapMode::Indent { width: 76 },
    ] {
        let rendered = mode.to_text();
        assert_eq!(WrapMode::parse(&rendered).unwrap(), mode);
    }
}

// ── WhitespaceRender text form ───────────────────────────────────────

#[test]
fn whitespace_render_parse_all_variants() {
    assert_eq!(
        WhitespaceRender::parse("none").unwrap(),
        WhitespaceRender::None
    );
    assert_eq!(
        WhitespaceRender::parse("all").unwrap(),
        WhitespaceRender::All
    );
    assert_eq!(
        WhitespaceRender::parse("trailing").unwrap(),
        WhitespaceRender::Trailing
    );
}

#[test]
fn whitespace_render_parse_case_insensitive() {
    assert_eq!(
        WhitespaceRender::parse("None").unwrap(),
        WhitespaceRender::None
    );
    assert_eq!(
        WhitespaceRender::parse("ALL").unwrap(),
        WhitespaceRender::All
    );
    assert_eq!(
        WhitespaceRender::parse("Trailing").unwrap(),
        WhitespaceRender::Trailing
    );
}

#[test]
fn whitespace_render_parse_error() {
    let err = WhitespaceRender::parse("always").unwrap_err();
    assert!(err.contains("always"), "error should contain input: {err}");
}

#[test]
fn whitespace_render_values_round_trip_through_parse() {
    // Every completion-offered value must parse.
    for v in WhitespaceRender::VALUES {
        assert!(
            WhitespaceRender::parse(v).is_ok(),
            "'{v}' should parse as WhitespaceRender"
        );
    }
}

#[test]
fn whitespace_render_to_text_round_trips_through_parse() {
    // `kind::Typed` (settings/kind.rs) reads this type back as
    // `Str(to_text())` for `(get-option "whitespace-space"|"whitespace-tab")`,
    // so `to_text` must round-trip through the `parse` that writes it.
    for variant in [
        WhitespaceRender::None,
        WhitespaceRender::All,
        WhitespaceRender::Trailing,
    ] {
        let rendered = variant.to_text();
        assert_eq!(WhitespaceRender::parse(&rendered).unwrap(), variant);
    }
}

// ── LineNumberStyle text form ────────────────────────────────────────

#[test]
fn line_number_style_parse_all_variants() {
    assert_eq!(
        LineNumberStyle::parse("absolute").unwrap(),
        LineNumberStyle::Absolute
    );
    assert_eq!(
        LineNumberStyle::parse("relative").unwrap(),
        LineNumberStyle::Relative
    );
    assert_eq!(
        LineNumberStyle::parse("hybrid").unwrap(),
        LineNumberStyle::Hybrid
    );
}

#[test]
fn line_number_style_parse_case_insensitive() {
    assert_eq!(
        LineNumberStyle::parse("Absolute").unwrap(),
        LineNumberStyle::Absolute
    );
    assert_eq!(
        LineNumberStyle::parse("RELATIVE").unwrap(),
        LineNumberStyle::Relative
    );
    assert_eq!(
        LineNumberStyle::parse("Hybrid").unwrap(),
        LineNumberStyle::Hybrid
    );
}

#[test]
fn line_number_style_parse_error() {
    let err = LineNumberStyle::parse("invalid").unwrap_err();
    assert!(err.contains("invalid"), "error should mention input: {err}");
    assert!(
        err.contains("absolute"),
        "error should list valid values: {err}"
    );
}

#[test]
fn line_number_style_to_text_round_trips_through_parse() {
    for v in LineNumberStyle::VALUES {
        let parsed = LineNumberStyle::parse(v).unwrap();
        assert_eq!(&parsed.to_text(), v);
    }
}

// ── LineEnding text form ─────────────────────────────────────────────────

#[test]
fn line_ending_names_round_trip() {
    for ending in [LineEnding::Lf, LineEnding::CrLf] {
        assert_eq!(LineEnding::parse(&ending.to_text()), Ok(ending));
    }
    assert_eq!(LineEnding::Lf.to_text(), "lf");
    assert_eq!(LineEnding::CrLf.to_text(), "crlf");
}

#[test]
fn line_ending_parse_rejects_unknown_names() {
    let err = LineEnding::parse("dos").unwrap_err();
    assert!(err.contains("lf") && err.contains("crlf"), "{err}");
}

#[test]
fn line_ending_parse_ignores_case() {
    assert_eq!(LineEnding::parse("LF"), Ok(LineEnding::Lf));
    assert_eq!(LineEnding::parse("CrLf"), Ok(LineEnding::CrLf));
}

#[test]
fn line_ending_values_parse_to_their_own_spelling() {
    for v in LineEnding::VALUES {
        assert_eq!(LineEnding::parse(v).map(|e| e.to_text()).as_deref(), Ok(*v));
    }
}

// ── StoreDirectory ───────────────────────────────────────────────────────

#[test]
fn store_directory_empty_is_the_named_folder_under_the_state_dir() {
    let state = AbsolutePath::rooted("/state/hume");
    let dirs = hume_platform::dirs::Dirs {
        state: Some(state.to_path_buf()),
        ..hume_platform::dirs::Dirs::none()
    };
    let parsed = StoreDirectory::parse("").unwrap();
    assert_eq!(parsed, StoreDirectory::Default);
    assert_eq!(parsed.to_text(), "");
    let work = AbsolutePath::rooted("/work");
    assert_eq!(
        parsed.location(&dirs, &work, "undo"),
        Ok(state.join("undo"))
    );
    assert_eq!(
        parsed.location(&dirs, &work, "sessions"),
        Ok(state.join("sessions"))
    );
}

#[test]
fn store_directory_default_has_no_location_without_a_state_dir() {
    let none = hume_platform::dirs::Dirs::none();
    assert!(
        StoreDirectory::Default
            .location(&none, &AbsolutePath::rooted("/work"), "undo")
            .is_err()
    );
}

#[test]
fn store_directory_takes_a_path_from_the_home_dir_or_the_root_and_reads_it_back() {
    let typed = StoreDirectory::parse("~/undo").unwrap();
    assert_eq!(typed.to_text(), "~/undo");
    let home = hume_platform::dirs::home_dir().expect("tests run with a home directory");
    let work = AbsolutePath::rooted("/work");
    assert_eq!(
        typed.location(&hume_platform::dirs::Dirs::none(), &work, "sessions"),
        Ok(home.join("undo"))
    );
    let rooted = AbsolutePath::rooted("/var/undo");
    let at = StoreDirectory::parse(&rooted.to_string_lossy()).unwrap();
    assert_eq!(
        at.location(&hume_platform::dirs::Dirs::none(), &work, "undo"),
        Ok(rooted.to_path_buf())
    );
}

#[test]
fn store_directory_rejects_a_relative_path() {
    assert!(StoreDirectory::parse("undo").is_err());
}

// ── Scope resolvers ─────────────────────────────────────────────────────────

#[test]
fn global_names_every_setting_with_a_global_value() {
    let global = |key| SettingKey::named(key).and_then(SettingKey::global);
    assert_eq!(
        global("scroll-margin"),
        Some(TableKey::Global(GlobalKey::scroll_margin))
    );
    assert_eq!(
        global("indent-width"),
        Some(TableKey::Buffer(BufferKey::indent_width))
    );
    assert_eq!(global("language"), None);
}

#[test]
fn buffer_names_an_override_or_an_option_the_buffer_stores() {
    let buffer = |key| SettingKey::named(key).and_then(SettingKey::buffer);
    assert_eq!(
        buffer("whitespace-tab"),
        Some(BufferTarget::Override(BufferKey::whitespace_tab))
    );
    assert_eq!(
        buffer("line-ending"),
        Some(BufferTarget::Intrinsic(BufferIntrinsicOption::LineEnding))
    );
    assert_eq!(buffer("statusline"), None);
}

#[test]
fn pane_names_only_a_setting_a_pane_overrides() {
    let pane = |key| SettingKey::named(key).and_then(SettingKey::pane);
    assert_eq!(pane("wrap-mode"), Some(PaneSetting::WrapMode));
    assert_eq!(pane("indent-width"), None);
    assert_eq!(pane("language"), None);
}

/// Every value `:set` value completion offers is one the setting accepts.
#[test]
fn every_completed_value_parses() {
    for key in TableKey::all() {
        for name in key.value_names() {
            let mut settings = EditorSettings::default();
            let written = write_global(key, &text(name), &mut settings, ConfigOrigin::User);
            assert_eq!(written, Ok(true), "'{}={name}'", key.name());
        }
    }
}

// ── Script values ───────────────────────────────────────────────────────────

/// Every script value type.
const SCRIPT_TYPES: [OptionType; 5] = [
    OptionType::Bool,
    OptionType::Int,
    OptionType::Str,
    OptionType::List,
    OptionType::Sections,
];

/// A script value of type `ty`.
fn script_value(ty: OptionType) -> OptionValue {
    match ty {
        OptionType::Bool => OptionValue::Bool(true),
        OptionType::Int => OptionValue::Int(1),
        OptionType::Str => OptionValue::Str("x".to_string()),
        OptionType::List => OptionValue::List(vec!["x".to_string()]),
        OptionType::Sections => OptionValue::Sections(StatuslineSections {
            left: Vec::new(),
            center: Vec::new(),
            right: Vec::new(),
        }),
    }
}

/// A setting takes a script value in the type it reads back as: its own
/// value written back is accepted, and a value of any other type is refused
/// naming both types.
#[test]
fn a_setting_takes_a_script_value_of_the_type_it_reads_back_as() {
    for key in TableKey::all() {
        let mut settings = EditorSettings::default();
        let own = read_global(key, &settings).value();
        let written = write_global(
            key,
            &OptionInput::Value(own.clone()),
            &mut settings,
            ConfigOrigin::User,
        );
        assert_eq!(written, Ok(true), "'{}' takes back {own:?}", key.name());
        let takes = own.option_type();
        for ty in SCRIPT_TYPES.into_iter().filter(|&ty| ty != takes) {
            let refused = write_global(
                key,
                &OptionInput::Value(script_value(ty)),
                &mut settings,
                ConfigOrigin::User,
            );
            assert_eq!(
                refused,
                Err(format!(
                    "invalid value for '{}': expected {}, got {}",
                    key.name(),
                    takes.name(),
                    ty.name()
                ))
            );
        }
    }
}

/// A script integer is held to the rule its text is.
#[test]
fn a_script_integer_is_checked_as_its_text_is() {
    for (key, n) in [
        ("scroll-margin", -1),
        ("jump-list-capacity", 0),
        ("indent-width", 0),
        ("indent-width", 256),
    ] {
        let write = |value: OptionInput| {
            write_global_for_test(
                key,
                &value,
                &mut EditorSettings::default(),
                ConfigOrigin::User,
            )
        };
        let typed = write(OptionInput::Value(OptionValue::Int(n)));
        assert!(typed.is_err(), "{key}={n}");
        assert_eq!(typed, write(text(&n.to_string())), "{key}={n}");
    }
    let mut settings = EditorSettings::default();
    write_global_for_test(
        "indent-width",
        &OptionInput::Value(OptionValue::Int(8)),
        &mut settings,
        ConfigOrigin::User,
    )
    .unwrap();
    assert_eq!(settings.indent_width, 8);
}

/// A list or hash whose contents did not decode is refused as the wrong type
/// by a setting that takes another type, and with its own error by one that
/// takes its type.
#[test]
fn an_unreadable_value_is_refused_by_type_before_its_contents() {
    let unreadable = OptionInput::Unreadable {
        ty: OptionType::List,
        error: "item 2: expected a string".to_string(),
    };
    let write = |key| {
        write_global_for_test(
            key,
            &unreadable,
            &mut EditorSettings::default(),
            ConfigOrigin::User,
        )
    };
    insta::assert_snapshot!(
        write("indent-width").unwrap_err(),
        @"invalid value for 'indent-width': expected an integer, got a list of strings"
    );
    insta::assert_snapshot!(
        write("shell").unwrap_err(),
        @"invalid value for 'shell': item 2: expected a string"
    );
}

#[test]
fn a_foreign_value_is_refused_naming_it() {
    let err = write_global_for_test(
        "indent-style",
        &OptionInput::Foreign("the symbol 'soft".to_string()),
        &mut EditorSettings::default(),
        ConfigOrigin::User,
    )
    .unwrap_err();
    insta::assert_snapshot!(
        err,
        @"invalid value for 'indent-style': expected a string, got the symbol 'soft"
    );
}
