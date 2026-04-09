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
//! ## Future layers
//!
//! The design accommodates a future EditorConfig layer between buffer overrides
//! and global settings without changing the public API: callers always go
//! through [`BufferOverrides`] accessors and [`EditorSettings`].

use engine::builtins::line_number::LineNumberStyle;
use engine::pane::{WhitespaceConfig, WrapMode};

use crate::auto_pairs::Pair;
use crate::ui::statusline::StatusLineConfig;

// ── EditorSettings ────────────────────────────────────────────────────────────

/// Global editor settings — the authoritative defaults for all configurable
/// editor behaviour.
///
/// Fields are grouped into *global-only* (no per-buffer override makes sense)
/// and *per-buffer-overridable* (a [`BufferOverrides`] on a document can
/// shadow them).
///
/// The [`Default`] impl exactly reproduces the values that were previously
/// hardcoded as constants across the codebase.
pub(crate) struct EditorSettings {
    // ── Global-only ──────────────────────────────────────────────────────────

    /// Rows to keep between the cursor and the top/bottom viewport edge.
    pub scroll_margin: usize,
    /// Columns to keep between the cursor and the left/right viewport edge.
    pub scroll_margin_h: usize,
    /// Lines to scroll per mouse-wheel notch.
    pub mouse_scroll_lines: usize,
    /// Whether mouse tracking is enabled at all.
    pub mouse_enabled: bool,
    /// Whether click-and-drag mouse selection is enabled (requires `mouse_enabled`).
    pub mouse_select: bool,
    /// Maximum number of entries kept in the jump list.
    pub jump_list_capacity: usize,
    /// Movements crossing more than this many lines are auto-recorded as jumps.
    pub jump_line_threshold: usize,
    /// Statusline layout configuration.
    pub statusline: StatusLineConfig,

    // ── Per-buffer-overridable ────────────────────────────────────────────────

    /// Tab stop width in columns.
    pub tab_width: u8,
    /// Line wrapping behaviour.
    pub wrap_mode: WrapMode,
    /// Gutter line-number display style.
    pub line_number_style: LineNumberStyle,
    /// Master switch for automatic bracket/quote pairing.
    pub auto_pairs_enabled: bool,
    /// The active auto-pair set. Indexed by open character.
    pub auto_pairs: Vec<Pair>,
    /// Whitespace indicator rendering configuration.
    pub whitespace: WhitespaceConfig,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            // Global-only — values previously hardcoded as module constants.
            scroll_margin: 3,
            scroll_margin_h: 5,
            mouse_scroll_lines: 3,
            mouse_enabled: true,
            mouse_select: false,
            jump_list_capacity: 100,
            jump_line_threshold: 5,
            statusline: StatusLineConfig::default(),

            // Per-buffer-overridable — values previously hardcoded in Editor::open().
            tab_width: 4,
            wrap_mode: WrapMode::Indent { width: 76 },
            line_number_style: LineNumberStyle::Hybrid,
            auto_pairs_enabled: true,
            auto_pairs: vec![
                Pair { open: '(', close: ')' },
                Pair { open: '[', close: ']' },
                Pair { open: '{', close: '}' },
                Pair { open: '"', close: '"' },
                Pair { open: '\'', close: '\'' },
                Pair { open: '`', close: '`' },
            ],
            whitespace: WhitespaceConfig::default(),
        }
    }
}

// ── BufferOverrides ───────────────────────────────────────────────────────────

/// Per-buffer setting overrides. All fields are `Option<T>`; `None` means
/// "inherit from the global [`EditorSettings`]".
///
/// Resolution is always lazy: call the accessor (e.g. [`Self::tab_width`])
/// with a `&EditorSettings` reference to get the effective value.
#[derive(Default)]
pub(crate) struct BufferOverrides {
    pub tab_width: Option<u8>,
    pub wrap_mode: Option<WrapMode>,
    pub line_number_style: Option<LineNumberStyle>,
    pub auto_pairs_enabled: Option<bool>,
    pub auto_pairs: Option<Vec<Pair>>,
    pub whitespace: Option<WhitespaceConfig>,
}

impl BufferOverrides {
    /// Effective tab width: buffer override → global default.
    pub(crate) fn tab_width(&self, global: &EditorSettings) -> u8 {
        self.tab_width.unwrap_or(global.tab_width)
    }

    /// Effective wrap mode: buffer override → global default.
    pub(crate) fn wrap_mode(&self, global: &EditorSettings) -> WrapMode {
        self.wrap_mode.clone().unwrap_or_else(|| global.wrap_mode.clone())
    }

    /// Effective line-number style: buffer override → global default.
    pub(crate) fn line_number_style(&self, global: &EditorSettings) -> LineNumberStyle {
        self.line_number_style
            .clone()
            .unwrap_or_else(|| global.line_number_style.clone())
    }

    /// Effective whitespace config: buffer override → global default.
    pub(crate) fn whitespace(&self, global: &EditorSettings) -> WhitespaceConfig {
        self.whitespace.clone().unwrap_or_else(|| global.whitespace.clone())
    }

    /// Effective auto-pairs config for this buffer: `(enabled, &pairs)`.
    ///
    /// Returns references to avoid a `Vec` allocation on every keystroke.
    /// The `enabled` flag and the pair list are resolved independently so a
    /// buffer can override just one without replacing the other.
    pub(crate) fn auto_pairs_ref<'a>(&'a self, global: &'a EditorSettings) -> (bool, &'a [Pair]) {
        let enabled = self.auto_pairs_enabled.unwrap_or(global.auto_pairs_enabled);
        let pairs: &[Pair] = match &self.auto_pairs {
            Some(p) => p.as_slice(),
            None => &global.auto_pairs,
        };
        (enabled, pairs)
    }
}

// ── :set parsing ─────────────────────────────────────────────────────────────

/// Apply a global setting mutation from a `:set global key=value` command.
///
/// Returns `Err(message)` on unknown key or invalid value.
pub(crate) fn apply_global(
    settings: &mut EditorSettings,
    key: &str,
    value: &str,
) -> Result<(), String> {
    match key {
        "scroll-margin" => {
            settings.scroll_margin = parse_usize(value, key)?;
        }
        "scroll-margin-h" => {
            settings.scroll_margin_h = parse_usize(value, key)?;
        }
        "mouse-scroll-lines" => {
            settings.mouse_scroll_lines = parse_usize(value, key)?;
        }
        "mouse-enabled" => {
            settings.mouse_enabled = parse_bool(value, key)?;
        }
        "mouse-select" => {
            settings.mouse_select = parse_bool(value, key)?;
        }
        "jump-list-capacity" => {
            settings.jump_list_capacity = parse_usize_nonzero(value, key)?;
        }
        "jump-line-threshold" => {
            settings.jump_line_threshold = parse_usize(value, key)?;
        }
        "tab-width" => {
            settings.tab_width = parse_tab_width(value)?;
        }
        "line-number-style" => {
            settings.line_number_style = value.parse()?;
        }
        "auto-pairs-enabled" => {
            settings.auto_pairs_enabled = parse_bool(value, key)?;
        }
        "whitespace-space" => {
            settings.whitespace.space = value.parse()?;
        }
        "whitespace-tab" => {
            settings.whitespace.tab = value.parse()?;
        }
        "whitespace-newline" => {
            settings.whitespace.newline = value.parse()?;
        }
        "wrap-mode" => {
            settings.wrap_mode = value.parse()?;
        }
        _ => return Err(format!("unknown setting '{key}'")),
    }
    Ok(())
}

/// Apply a per-buffer setting override from a `:set buffer key=value` command.
///
/// Returns `Err(message)` on unknown key, global-only key, or invalid value.
pub(crate) fn apply_buffer(
    overrides: &mut BufferOverrides,
    key: &str,
    value: &str,
) -> Result<(), String> {
    match key {
        "tab-width" => {
            overrides.tab_width = Some(parse_tab_width(value)?);
        }
        "wrap-mode" => {
            overrides.wrap_mode = Some(value.parse()?);
        }
        "line-number-style" => {
            overrides.line_number_style = Some(value.parse()?);
        }
        "auto-pairs-enabled" => {
            overrides.auto_pairs_enabled = Some(parse_bool(value, key)?);
        }
        "whitespace-space" => {
            // Ensure override struct exists; patch just the one field.
            let ws = overrides.whitespace.get_or_insert_with(WhitespaceConfig::default);
            ws.space = value.parse()?;
        }
        "whitespace-tab" => {
            let ws = overrides.whitespace.get_or_insert_with(WhitespaceConfig::default);
            ws.tab = value.parse()?;
        }
        "whitespace-newline" => {
            let ws = overrides.whitespace.get_or_insert_with(WhitespaceConfig::default);
            ws.newline = value.parse()?;
        }
        // Global-only settings
        "scroll-margin"
        | "scroll-margin-h"
        | "mouse-scroll-lines"
        | "mouse-enabled"
        | "mouse-select"
        | "jump-list-capacity"
        | "jump-line-threshold" => {
            return Err(format!("'{key}' is a global-only setting — use :set global {key}=…"));
        }
        _ => return Err(format!("unknown setting '{key}'")),
    }
    Ok(())
}

// ── Value parsers ─────────────────────────────────────────────────────────────

fn parse_usize(value: &str, key: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid value for '{key}': expected a non-negative integer, got '{value}'"))
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

    // ── apply_global ──────────────────────────────────────────────────────────

    #[test]
    fn set_global_scroll_margin() {
        let mut s = EditorSettings::default();
        apply_global(&mut s, "scroll-margin", "1").unwrap();
        assert_eq!(s.scroll_margin, 1);
    }

    #[test]
    fn set_global_tab_width() {
        let mut s = EditorSettings::default();
        apply_global(&mut s, "tab-width", "8").unwrap();
        assert_eq!(s.tab_width, 8);
    }

    #[test]
    fn set_global_line_number_style() {
        let mut s = EditorSettings::default();
        apply_global(&mut s, "line-number-style", "relative").unwrap();
        assert_eq!(s.line_number_style, LineNumberStyle::Relative);
    }

    #[test]
    fn set_global_wrap_mode_none() {
        let mut s = EditorSettings::default();
        apply_global(&mut s, "wrap-mode", "none").unwrap();
        assert_eq!(s.wrap_mode, WrapMode::None);
    }

    #[test]
    fn set_global_wrap_mode_indent() {
        let mut s = EditorSettings::default();
        apply_global(&mut s, "wrap-mode", "indent:80").unwrap();
        assert_eq!(s.wrap_mode, WrapMode::Indent { width: 80 });
    }

    #[test]
    fn set_global_auto_pairs_enabled() {
        let mut s = EditorSettings::default();
        apply_global(&mut s, "auto-pairs-enabled", "false").unwrap();
        assert!(!s.auto_pairs_enabled);
    }

    #[test]
    fn set_global_unknown_key_errors() {
        let mut s = EditorSettings::default();
        assert!(apply_global(&mut s, "nonexistent", "42").is_err());
    }

    #[test]
    fn set_global_invalid_value_errors() {
        let mut s = EditorSettings::default();
        assert!(apply_global(&mut s, "scroll-margin", "abc").is_err());
    }

    #[test]
    fn set_global_tab_width_zero_errors() {
        let mut s = EditorSettings::default();
        assert!(apply_global(&mut s, "tab-width", "0").is_err());
    }

    // ── apply_buffer ─────────────────────────────────────────────────────────

    #[test]
    fn set_buffer_tab_width() {
        let global = EditorSettings::default();
        let mut ov = BufferOverrides::default();
        apply_buffer(&mut ov, "tab-width", "8").unwrap();
        assert_eq!(ov.tab_width(&global), 8);
    }

    #[test]
    fn set_buffer_wrap_mode() {
        let global = EditorSettings::default();
        let mut ov = BufferOverrides::default();
        apply_buffer(&mut ov, "wrap-mode", "none").unwrap();
        assert_eq!(ov.wrap_mode(&global), WrapMode::None);
    }

    #[test]
    fn set_buffer_global_only_setting_errors() {
        let mut ov = BufferOverrides::default();
        let err = apply_buffer(&mut ov, "scroll-margin", "3").unwrap_err();
        assert!(err.contains("global-only"), "expected 'global-only' in error: {err}");
    }

    #[test]
    fn set_global_tab_width_propagates_to_unoverridden_buffer() {
        let mut global = EditorSettings::default();
        let ov = BufferOverrides::default();
        apply_global(&mut global, "tab-width", "2").unwrap();
        // Buffer has no override, so it inherits the new global value.
        assert_eq!(ov.tab_width(&global), 2);
    }
}
