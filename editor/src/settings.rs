//! Centralized editor settings — the single source of truth for all
//! configurable editor behaviour.
//!
//! ## Layering
//!
//! ```text
//! hardcoded default → EditorSettings (global) → BufferOverrides (per-buffer)
//! ```
//!
//! [`EditorSettings`] holds concrete values for every setting. Its [`Default`]
//! impl reproduces today's hardcoded defaults, so the editor behaves identically
//! with no explicit configuration.
//!
//! [`BufferOverrides`] lives on each [`crate::core::document::Document`] and
//! stores `Option<T>` for every per-buffer-overridable setting. `None` means
//! "inherit from global". Resolution happens at call time via the accessor
//! methods on [`BufferOverrides`] — no pre-merged copy is kept.
//!
//! ## Adding a setting
//!
//! Most settings are defined in a single [`define_settings!`] invocation that
//! generates [`EditorSettings`], [`BufferOverrides`], their `Default` impls,
//! accessor methods, and the [`apply_setting`] dispatch. Adding a simple
//! setting requires one entry in the macro and nothing else.
//!
//! Settings with non-trivial resolution (`auto_pairs_ref`, whitespace
//! sub-fields) are handled manually below the macro invocation.
//!
//! ## Future layers
//!
//! The design accommodates a future EditorConfig layer between buffer overrides
//! and global settings without changing the public API: callers always go
//! through [`BufferOverrides`] accessors and [`EditorSettings`].

use engine::builtins::line_number::LineNumberStyle;
use engine::pane::{WhitespaceConfig, WrapMode};

use crate::auto_pairs::Pair;
use crate::ui::statusline::StatusLineConfig;

// ── SettingScope ──────────────────────────────────────────────────────────────

/// Scope for a `:set` command.
///
/// `Global` applies to editor-wide defaults (written to [`EditorSettings`]).
/// `Buffer` overrides a setting for the active buffer only (written to
/// [`BufferOverrides`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingScope {
    Global,
    Buffer,
}

// ── Parser helper ─────────────────────────────────────────────────────────────

/// Dispatch from a parser-kind token to the actual parse call.
///
/// All arms return `Result<T, String>`. Used inside `apply_setting` (generated
/// by `define_settings!`) where `value` and `key` are in scope.
macro_rules! parse_setting {
    ($value:expr, $key:expr, bool)          => { parse_bool($value, $key) };
    ($value:expr, $key:expr, usize)         => { parse_usize($value, $key) };
    ($value:expr, $key:expr, usize_nonzero) => { parse_usize_nonzero($value, $key) };
    ($value:expr, $key:expr, tab_width)     => { parse_tab_width($value) };
    ($value:expr, $key:expr, from_str)      => { $value.parse() };
}

// ── Settings definition ───────────────────────────────────────────────────────

/// Generate [`EditorSettings`], [`BufferOverrides`], and [`apply_setting`]
/// from a single source of truth.
///
/// ## Sections
///
/// - `global { … }` — global-only settings with a `:set` key; format:
///   `"key" => field: Type = default, parser: kind;`
/// - `buffer { … }` — per-buffer-overridable settings with a `:set` key;
///   same format, generates both a global field and a buffer override
/// - `extra_global { … }` — extra global-only fields without a `:set` key;
///   format: `field: Type = default;`
/// - `extra_buffer { … }` — extra per-buffer fields without a `:set` key;
///   format: `field: Type = global_default;` (buffer default is always `None`)
///
/// ## Parser kinds
///
/// | Token | Function |
/// |-------|----------|
/// | `bool` | `parse_bool(value, key)` |
/// | `usize` | `parse_usize(value, key)` |
/// | `usize_nonzero` | `parse_usize_nonzero(value, key)` |
/// | `tab_width` | `parse_tab_width(value)` |
/// | `from_str` | `value.parse()` (type inferred from field) |
macro_rules! define_settings {
    (
        global {
            $( $gkey:literal => $gname:ident : $gtype:ty = $gdefault:expr, parser: $gparser:ident; )*
        }
        buffer {
            $( $bkey:literal => $bname:ident : $btype:ty = $bdefault:expr, parser: $bparser:ident; )*
        }
        extra_global {
            $( $egname:ident : $egtype:ty = $egdefault:expr; )*
        }
        extra_buffer {
            $( $ebname:ident : $ebtype:ty = $ebdefault:expr; )*
        }
    ) => {

        // ── EditorSettings ────────────────────────────────────────────────────

        /// Global editor settings — the authoritative defaults for all
        /// configurable editor behaviour.
        ///
        /// The [`Default`] impl exactly reproduces the values that were
        /// previously hardcoded as constants across the codebase.
        pub(crate) struct EditorSettings {
            $( pub $gname: $gtype, )*
            $( pub $bname: $btype, )*
            $( pub $egname: $egtype, )*
            $( pub $ebname: $ebtype, )*
        }

        impl Default for EditorSettings {
            fn default() -> Self {
                Self {
                    $( $gname: $gdefault, )*
                    $( $bname: $bdefault, )*
                    $( $egname: $egdefault, )*
                    $( $ebname: $ebdefault, )*
                }
            }
        }

        // ── BufferOverrides ───────────────────────────────────────────────────

        /// Per-buffer setting overrides. All fields are `Option<T>`; `None`
        /// means "inherit from the global [`EditorSettings`]".
        ///
        /// Resolution is always lazy: call the accessor (e.g.
        /// [`Self::tab_width`]) with a `&EditorSettings` reference.
        #[derive(Default)]
        pub(crate) struct BufferOverrides {
            $( pub $bname: Option<$btype>, )*
            $( pub $ebname: Option<$ebtype>, )*
        }

        impl BufferOverrides {
            $(
                /// Effective value: buffer override → global default.
                pub(crate) fn $bname(&self, global: &EditorSettings) -> $btype {
                    self.$bname.clone().unwrap_or_else(|| global.$bname.clone())
                }
            )*
        }

        // ── apply_setting ─────────────────────────────────────────────────────

        /// Apply a setting mutation from a `:set scope key=value` command.
        ///
        /// - `Global` scope writes to `settings` (always valid for all keys)
        /// - `Buffer` scope writes to `overrides` (rejected for global-only
        ///   keys)
        ///
        /// Returns `Err(message)` on unknown key, wrong-scope key, or invalid
        /// value.
        pub(crate) fn apply_setting(
            scope: SettingScope,
            key: &str,
            value: &str,
            settings: &mut EditorSettings,
            overrides: &mut BufferOverrides,
        ) -> Result<(), String> {
            match (scope, key) {
                // Global-only settings: valid only with Global scope
                $( (SettingScope::Global, $gkey) => {
                    settings.$gname = parse_setting!(value, key, $gparser)?;
                } )*
                // Per-buffer settings: Global scope writes to EditorSettings
                $( (SettingScope::Global, $bkey) => {
                    settings.$bname = parse_setting!(value, key, $bparser)?;
                } )*
                // Per-buffer settings: Buffer scope writes to override
                $( (SettingScope::Buffer, $bkey) => {
                    overrides.$bname = Some(parse_setting!(value, key, $bparser)?);
                } )*
                // Global-only settings rejected when scope is Buffer
                $( (SettingScope::Buffer, $gkey) => {
                    return Err(format!(
                        "'{key}' is a global-only setting — use :set global {key}=…"
                    ));
                } )*
                // Whitespace sub-fields — patch one sub-field at a time to let
                // buffers override space/tab/newline independently.
                (SettingScope::Global, "whitespace-space") => {
                    settings.whitespace.space = value.parse()?;
                }
                (SettingScope::Global, "whitespace-tab") => {
                    settings.whitespace.tab = value.parse()?;
                }
                (SettingScope::Global, "whitespace-newline") => {
                    settings.whitespace.newline = value.parse()?;
                }
                (SettingScope::Buffer, "whitespace-space") => {
                    overrides
                        .whitespace
                        .get_or_insert_with(WhitespaceConfig::default)
                        .space = value.parse()?;
                }
                (SettingScope::Buffer, "whitespace-tab") => {
                    overrides
                        .whitespace
                        .get_or_insert_with(WhitespaceConfig::default)
                        .tab = value.parse()?;
                }
                (SettingScope::Buffer, "whitespace-newline") => {
                    overrides
                        .whitespace
                        .get_or_insert_with(WhitespaceConfig::default)
                        .newline = value.parse()?;
                }
                _ => return Err(format!("unknown setting '{key}'")),
            }
            Ok(())
        }
    };
}

define_settings! {
    global {
        "scroll-margin"       => scroll_margin:       usize = 3,    parser: usize;
        "scroll-margin-h"     => scroll_margin_h:     usize = 5,    parser: usize;
        "mouse-scroll-lines"  => mouse_scroll_lines:  usize = 3,    parser: usize;
        "mouse-enabled"       => mouse_enabled:       bool  = true, parser: bool;
        "mouse-select"        => mouse_select:        bool  = false, parser: bool;
        "jump-list-capacity"  => jump_list_capacity:  usize = 100,  parser: usize_nonzero;
        "jump-line-threshold" => jump_line_threshold: usize = 5,    parser: usize;
    }
    buffer {
        "tab-width"          => tab_width:          u8              = 4,
            parser: tab_width;
        "wrap-mode"          => wrap_mode:          WrapMode        = WrapMode::Indent { width: 76 },
            parser: from_str;
        "line-number-style"  => line_number_style:  LineNumberStyle = LineNumberStyle::Hybrid,
            parser: from_str;
        "auto-pairs-enabled" => auto_pairs_enabled: bool            = true,
            parser: bool;
    }
    extra_global {
        statusline: StatusLineConfig = StatusLineConfig::default();
    }
    extra_buffer {
        auto_pairs: Vec<Pair> = vec![
            Pair { open: '(', close: ')' },
            Pair { open: '[', close: ']' },
            Pair { open: '{', close: '}' },
            Pair { open: '"',  close: '"'  },
            Pair { open: '\'', close: '\'' },
            Pair { open: '`',  close: '`'  },
        ];
        whitespace: WhitespaceConfig = WhitespaceConfig::default();
    }
}

// ── BufferOverrides: manual accessors ─────────────────────────────────────────

impl BufferOverrides {
    /// Effective whitespace config: buffer override → global default.
    pub(crate) fn whitespace(&self, global: &EditorSettings) -> WhitespaceConfig {
        self.whitespace.clone().unwrap_or_else(|| global.whitespace.clone())
    }

    /// Effective auto-pairs config for this buffer: `(enabled, &pairs)`.
    ///
    /// Returns references to avoid a `Vec` allocation on every keystroke.
    /// The `enabled` flag and the pair list are resolved independently so a
    /// buffer can override just one without replacing the other.
    pub(crate) fn auto_pairs_ref<'a>(
        &'a self,
        global: &'a EditorSettings,
    ) -> (bool, &'a [Pair]) {
        let enabled = self.auto_pairs_enabled(global);
        let pairs: &[Pair] = match &self.auto_pairs {
            Some(p) => p.as_slice(),
            None => &global.auto_pairs,
        };
        (enabled, pairs)
    }
}

// ── Value parsers ─────────────────────────────────────────────────────────────

fn parse_usize(value: &str, key: &str) -> Result<usize, String> {
    value.parse::<usize>().map_err(|_| {
        format!("invalid value for '{key}': expected a non-negative integer, got '{value}'")
    })
}

fn parse_usize_nonzero(value: &str, key: &str) -> Result<usize, String> {
    let n = parse_usize(value, key)?;
    if n == 0 {
        return Err(format!("invalid value for '{key}': must be at least 1"));
    }
    Ok(n)
}

fn parse_bool(value: &str, key: &str) -> Result<bool, String> {
    match value {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(format!("invalid value for '{key}': expected true/false, got '{value}'")),
    }
}

fn parse_tab_width(value: &str) -> Result<u8, String> {
    let n: u8 = value
        .parse()
        .map_err(|_| format!("invalid tab-width: expected 1–255, got '{value}'"))?;
    if n == 0 {
        return Err("invalid tab-width: must be at least 1".into());
    }
    Ok(n)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Default values match previous hardcoded constants ─────────────────────

    #[test]
    fn editor_settings_default_matches_old_constants() {
        let s = EditorSettings::default();
        assert_eq!(s.scroll_margin, 3);
        assert_eq!(s.scroll_margin_h, 5);
        assert_eq!(s.mouse_scroll_lines, 3);
        assert!(s.mouse_enabled);
        assert!(!s.mouse_select);
        assert_eq!(s.jump_list_capacity, 100);
        assert_eq!(s.jump_line_threshold, 5);
        assert_eq!(s.tab_width, 4);
        assert_eq!(s.wrap_mode, WrapMode::Indent { width: 76 });
        assert_eq!(s.line_number_style, LineNumberStyle::Hybrid);
        assert!(s.auto_pairs_enabled);
    }

    #[test]
    fn buffer_overrides_default_is_all_none() {
        let ov = BufferOverrides::default();
        assert!(ov.tab_width.is_none());
        assert!(ov.wrap_mode.is_none());
        assert!(ov.line_number_style.is_none());
        assert!(ov.auto_pairs_enabled.is_none());
        assert!(ov.auto_pairs.is_none());
        assert!(ov.whitespace.is_none());
    }

    // ── Resolution: override present → returns override value ─────────────────

    #[test]
    fn resolution_override_wins_over_global() {
        let global = EditorSettings::default();
        let ov = BufferOverrides { tab_width: Some(8), ..Default::default() };
        assert_eq!(ov.tab_width(&global), 8);
    }

    #[test]
    fn resolution_wrap_mode_override_wins() {
        let global = EditorSettings::default();
        let ov = BufferOverrides { wrap_mode: Some(WrapMode::None), ..Default::default() };
        assert_eq!(ov.wrap_mode(&global), WrapMode::None);
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
    fn resolution_falls_back_to_global_wrap_mode() {
        let global = EditorSettings::default();
        let ov = BufferOverrides::default();
        assert_eq!(ov.wrap_mode(&global), global.wrap_mode);
    }

    // ── Auto-pairs resolution ─────────────────────────────────────────────────

    #[test]
    fn auto_pairs_override_enabled_only() {
        let global = EditorSettings::default();
        let ov = BufferOverrides { auto_pairs_enabled: Some(false), ..Default::default() };
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
        apply_setting(SettingScope::Buffer, key, value, &mut s, &mut ov)?;
        Ok(ov)
    }

    #[test]
    fn set_global_scroll_margin() {
        assert_eq!(global("scroll-margin", "1").unwrap().scroll_margin, 1);
    }

    #[test]
    fn set_global_scroll_margin_h() {
        assert_eq!(global("scroll-margin-h", "10").unwrap().scroll_margin_h, 10);
    }

    #[test]
    fn set_global_mouse_scroll_lines() {
        assert_eq!(global("mouse-scroll-lines", "5").unwrap().mouse_scroll_lines, 5);
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
        assert_eq!(global("jump-list-capacity", "50").unwrap().jump_list_capacity, 50);
    }

    #[test]
    fn set_global_jump_list_capacity_zero_errors() {
        assert!(global("jump-list-capacity", "0").is_err());
    }

    #[test]
    fn set_global_jump_line_threshold() {
        assert_eq!(global("jump-line-threshold", "10").unwrap().jump_line_threshold, 10);
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
    fn set_global_line_number_style() {
        assert_eq!(
            global("line-number-style", "relative").unwrap().line_number_style,
            LineNumberStyle::Relative,
        );
    }

    #[test]
    fn set_global_wrap_mode_none() {
        assert_eq!(global("wrap-mode", "none").unwrap().wrap_mode, WrapMode::None);
    }

    #[test]
    fn set_global_wrap_mode_indent() {
        assert_eq!(
            global("wrap-mode", "indent:80").unwrap().wrap_mode,
            WrapMode::Indent { width: 80 },
        );
    }

    #[test]
    fn set_global_auto_pairs_enabled() {
        assert!(!global("auto-pairs-enabled", "false").unwrap().auto_pairs_enabled);
    }

    #[test]
    fn set_global_whitespace_space() {
        assert_eq!(
            global("whitespace-space", "all").unwrap().whitespace.space,
            engine::pane::WhitespaceRender::All,
        );
    }

    #[test]
    fn set_global_whitespace_tab() {
        assert_eq!(
            global("whitespace-tab", "trailing").unwrap().whitespace.tab,
            engine::pane::WhitespaceRender::Trailing,
        );
    }

    #[test]
    fn set_global_whitespace_newline() {
        assert_eq!(
            global("whitespace-newline", "all").unwrap().whitespace.newline,
            engine::pane::WhitespaceRender::All,
        );
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
        assert!(global("tab-width", "").is_err());
        assert!(global("mouse-enabled", "").is_err());
    }

    // ── apply_setting: Buffer scope ───────────────────────────────────────────

    #[test]
    fn set_buffer_tab_width() {
        let global = EditorSettings::default();
        let ov = buffer("tab-width", "8").unwrap();
        assert_eq!(ov.tab_width(&global), 8);
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
            engine::builtins::line_number::LineNumberStyle::Absolute,
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
    fn set_buffer_whitespace_space() {
        let global = EditorSettings::default();
        let ov = buffer("whitespace-space", "all").unwrap();
        assert_eq!(ov.whitespace(&global).space, engine::pane::WhitespaceRender::All);
    }

    #[test]
    fn set_buffer_whitespace_tab() {
        let global = EditorSettings::default();
        let ov = buffer("whitespace-tab", "trailing").unwrap();
        assert_eq!(ov.whitespace(&global).tab, engine::pane::WhitespaceRender::Trailing);
    }

    #[test]
    fn set_buffer_whitespace_newline() {
        let global = EditorSettings::default();
        let ov = buffer("whitespace-newline", "all").unwrap();
        assert_eq!(ov.whitespace(&global).newline, engine::pane::WhitespaceRender::All);
    }

    #[test]
    fn set_buffer_whitespace_fields_are_independent() {
        // Setting whitespace-space should not touch tab or newline.
        let global = EditorSettings::default();
        let ov = buffer("whitespace-space", "all").unwrap();
        let ws = ov.whitespace(&global);
        assert_eq!(ws.space, engine::pane::WhitespaceRender::All);
        assert_eq!(ws.tab, engine::pane::WhitespaceRender::None);
        assert_eq!(ws.newline, engine::pane::WhitespaceRender::None);
    }

    #[test]
    fn set_buffer_global_only_setting_errors() {
        let mut s = EditorSettings::default();
        let mut ov = BufferOverrides::default();
        let err = apply_setting(SettingScope::Buffer, "scroll-margin", "3", &mut s, &mut ov)
            .unwrap_err();
        assert!(err.contains("global-only"), "expected 'global-only' in error: {err}");
    }

    #[test]
    fn set_buffer_global_only_all_keys_error() {
        let mut s = EditorSettings::default();
        let mut ov = BufferOverrides::default();
        for key in [
            "scroll-margin",
            "scroll-margin-h",
            "mouse-scroll-lines",
            "mouse-enabled",
            "mouse-select",
            "jump-list-capacity",
            "jump-line-threshold",
        ] {
            let err = apply_setting(SettingScope::Buffer, key, "1", &mut s, &mut ov)
                .unwrap_err();
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
        // Buffer has no override, so it inherits the new global value.
        assert_eq!(ov.tab_width(&global), 2);
    }
}
