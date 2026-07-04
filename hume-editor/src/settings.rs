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
//! [`BufferOverrides`] lives on each [`crate::editor::buffer::Text`] and
//! stores `Option<T>` for every per-buffer-overridable setting. `None` means
//! "inherit from global". Resolution happens at call time via the accessor
//! methods on [`BufferOverrides`] — no pre-merged copy is kept.
//!
//! ## Adding a setting
//!
//! Most settings are defined in a single [`define_settings!`] invocation that
//! generates [`EditorSettings`], [`BufferOverrides`], their `Default` impls,
//! accessor methods, and the [`apply_setting`]/`setting_scopes` dispatch.
//! Adding a simple setting requires one entry in the macro and nothing else.
//! `scope: [...]` on that entry is the single source of truth for which
//! `:set` scopes (`global`/`buffer`/`pane`) the key accepts — `typed_set`
//! (`editor::commands::typed_file`) looks it up via `setting_scopes(key)`
//! rather than special-casing scopes per key. A setting placed in the
//! `buffer{}` section always accepts `["global", "buffer"]`; whether `"pane"`
//! is also listed is independent of section placement (see `wrap-mode`
//! below) and requires a matching `if key == "..."` write arm in `typed_set`,
//! since `apply_setting` has no pane-storage concept at all.
//!
//! `language` is the one setting with no macro entry at all: it has no
//! global default (folding it in would let `:set global language=…`
//! silently succeed) and its write path needs `Editor`-level access
//! (`OnLanguageSet` hook, registry lookup) that `apply_setting`'s
//! `(&mut EditorSettings, &mut BufferOverrides)` signature doesn't have.
//! It stays a small, explicit special case in `typed_set`.
//!
//! Settings with non-trivial resolution (`auto_pairs_ref`, whitespace
//! sub-fields) are handled manually below the macro invocation.

use std::fmt;
use std::str::FromStr;

use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};

use crate::auto_pairs::Pair;
use crate::ui::statusline::{StatusElement, StatusLineConfig};

// ── TabStyle ──────────────────────────────────────────────────────────────────

/// What the Tab key inserts in Insert mode.
///
/// `Hard` inserts a literal `\t` character; `Soft` inserts enough spaces to
/// reach the next tab stop (governed by `tab-width`). This is the single knob
/// — there is no separate "shiftwidth" or "softtabstop": `tab-width` is the
/// only width, used for both rendering and Tab-key spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabStyle {
    /// Tab key inserts one `\t` character per press.
    #[default]
    Hard,
    /// Tab key inserts spaces up to the next tab stop.
    Soft,
}

impl TabStyle {
    /// The wire-format strings `FromStr` accepts — the single source
    /// `:set buffer tab-style=<Tab>` completion mirrors, so the two can never
    /// drift out of sync.
    pub const VALUES: &'static [&'static str] = &["hard", "soft"];
}

impl fmt::Display for TabStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hard => f.write_str("hard"),
            Self::Soft => f.write_str("soft"),
        }
    }
}

impl FromStr for TabStyle {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "hard" => Ok(Self::Hard),
            "soft" => Ok(Self::Soft),
            _ => Err(format!(
                "invalid tab-style: expected 'hard' or 'soft', got '{s}'"
            )),
        }
    }
}

// ── SettingScope ──────────────────────────────────────────────────────────────

/// Scope for a `:set` command.
///
/// `Global` applies to editor-wide defaults (written to [`EditorSettings`]).
/// `Text` overrides a setting for the active buffer only (written to
/// [`BufferOverrides`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingScope {
    Global,
    Text,
}

// ── Parser helper ─────────────────────────────────────────────────────────────

/// Dispatch from a parser-kind token to the actual parse call.
///
/// All arms return `Result<T, String>`. Used inside `apply_setting` (generated
/// by `define_settings!`) where `value` and `key` are in scope.
macro_rules! parse_setting {
    ($value:expr, $key:expr, bool) => {
        parse_bool($value, $key)
    };
    ($value:expr, $key:expr, usize) => {
        parse_usize($value, $key)
    };
    ($value:expr, $key:expr, usize_nonzero) => {
        parse_usize_nonzero($value, $key)
    };
    ($value:expr, $key:expr, tab_width) => {
        parse_tab_width($value)
    };
    ($value:expr, $key:expr, from_str) => {
        $value.parse()
    };
    ($value:expr, $key:expr, string) => {
        Ok::<String, String>(($value).to_owned())
    };
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
/// - `extra_global { … }` — extra fields on `EditorSettings` only, no `:set`
///   key; format: `field: Type = default;`
/// - `extra_buffer { … }` — extra fields on both structs, no `:set` key;
///   format: `field: Type = global_default;` (buffer default is always `None`)
/// - `override_only { … }` — extra `Option<T>` fields on `BufferOverrides`
///   only (no corresponding `EditorSettings` field); format: `field: Type;`
///   Resolution is handled manually in a separate `impl BufferOverrides` block.
/// - `manual_keys { … }` — `:set` keys whose values need custom resolution
///   (not a plain field write) and so get a hand-written `apply_setting` arm
///   below the macro invocation; format: `"key" => [scope, scope, ...];`.
///   This is the sole source for those keys' entries in [`setting_scopes`]
///   and [`all_setting_keys`] — the hand-written `apply_setting` arm is the
///   only thing that can't be generated from it.
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
            $( $gkey:literal => $gname:ident : $gtype:ty = $gdefault:expr, scope: [$($gscope:literal),+], parser: $gparser:ident; )*
        }
        buffer {
            $( $bkey:literal => $bname:ident : $btype:ty = $bdefault:expr, scope: [$($bscope:literal),+], parser: $bparser:ident; )*
        }
        extra_global {
            $( $egname:ident : $egtype:ty = $egdefault:expr; )*
        }
        extra_buffer {
            $( $ebname:ident : $ebtype:ty = $ebdefault:expr; )*
        }
        override_only {
            $( $ooname:ident : $ootype:ty; )*
        }
        manual_keys {
            $( $mkey:literal => [$($mscope:literal),+]; )*
        }
    ) => {

        // ── EditorSettings ────────────────────────────────────────────────────

        /// Global editor settings — the authoritative defaults for all
        /// configurable editor behaviour.
        ///
        /// The [`Default`] impl is the single source of truth for these
        /// default values.
        #[derive(Clone)]
        pub struct EditorSettings {
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
        pub struct BufferOverrides {
            $( pub $bname: Option<$btype>, )*
            $( pub $ebname: Option<$ebtype>, )*
            $( pub $ooname: Option<$ootype>, )*
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
        /// - `Text` scope writes to `overrides` (rejected for global-only
        ///   keys)
        ///
        /// Returns `Err(message)` on unknown key, wrong-scope key, or invalid
        /// value.
        pub fn apply_setting(
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
                // Per-buffer settings: Text scope writes to override
                $( (SettingScope::Text, $bkey) => {
                    overrides.$bname = Some(parse_setting!(value, key, $bparser)?);
                } )*
                // Global-only settings rejected when scope is Text
                $( (SettingScope::Text, $gkey) => {
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
                (SettingScope::Text, "whitespace-space") => {
                    overrides.whitespace_space = Some(value.parse()?);
                }
                (SettingScope::Text, "whitespace-tab") => {
                    overrides.whitespace_tab = Some(value.parse()?);
                }
                (SettingScope::Text, "whitespace-newline") => {
                    overrides.whitespace_newline = Some(value.parse()?);
                }
                // Statusline config — global-only; three sections separated by `|`,
                // each a comma-separated list of StatusElement names (may be empty).
                (SettingScope::Global, "statusline") => {
                    settings.statusline = parse_statusline(value)?;
                }
                (SettingScope::Text, "statusline") => {
                    return Err("'statusline' is a global-only setting — use :set global statusline=…".to_string());
                }
                _ => return Err(format!("unknown setting '{key}'")),
            }
            Ok(())
        }

        // ── setting_scopes ──────────────────────────────────────────────────────

        /// The `:set` scopes a setting accepts (`"global"`, `"buffer"`, `"pane"`),
        /// as declared by its `scope: [...]` list in the [`define_settings!`]
        /// invocation below. Empty for any key not declared there — notably
        /// `"language"`, which has no generic storage and is handled entirely by
        /// `typed_set`'s own special case, never through this table.
        pub(crate) fn setting_scopes(key: &str) -> &'static [&'static str] {
            match key {
                $( $gkey => &[$($gscope),+], )*
                $( $bkey => &[$($bscope),+], )*
                $( $mkey => &[$($mscope),+], )*
                _ => &[],
            }
        }

        // ── all_setting_keys ───────────────────────────────────────────────────

        /// Every setting key with a `:set` wire format — the union of the
        /// `global`/`buffer` macro entries and the `manual_keys` entries
        /// (`whitespace-*`, `statusline`). Notably **excludes** `"language"`,
        /// which has no macro entry and is surfaced only when the completer
        /// knows the scope is `"buffer"` (its sole valid scope). Used by
        /// [`crate::editor::completion::SetCompleter`] to enumerate key
        /// candidates, filtered further by [`setting_scopes`] against the
        /// chosen scope.
        pub(crate) fn all_setting_keys() -> &'static [&'static str] {
            &[$($gkey,)* $($bkey,)* $($mkey,)*]
        }
    };
}

define_settings! {
    global {
        "scrolloff" => scrolloff: usize = 3,
            scope: ["global"],
            parser: usize;
        "mouse-scroll-lines" => mouse_scroll_lines: usize = 3,
            scope: ["global"],
            parser: usize;
        "mouse-enabled" => mouse_enabled: bool = true,
            scope: ["global"],
            parser: bool;
        "mouse-select" => mouse_select: bool = false,
            scope: ["global"],
            parser: bool;
        "jump-list-capacity" => jump_list_capacity: usize = 100,
            scope: ["global"],
            parser: usize_nonzero;
        "jump-line-threshold" => jump_line_threshold: usize = 5,
            scope: ["global"],
            parser: usize;
        "history-capacity" => history_capacity: usize = 100,
            scope: ["global"],
            parser: usize_nonzero;
        "steel-init-budget-ms" => steel_init_budget_ms: usize = 10_000,
            scope: ["global"],
            parser: usize_nonzero;
        "steel-command-budget-ms" => steel_command_budget_ms: usize = 1_000,
            scope: ["global"],
            parser: usize_nonzero;
        "popup-border" => popup_border: bool = true,
            scope: ["global"],
            parser: bool;
        "pane-dividers" => pane_dividers: bool = true,
            scope: ["global"],
            parser: bool;
        "theme" => theme: String = String::new(),
            scope: ["global"],
            parser: string;
        "syntax-highlight-max-bytes" => syntax_highlight_max_bytes: usize = 1_048_576,
            scope: ["global"],
            parser: usize_nonzero;
        // Global-only *storage*: seeds new panes' `Pane::wrap_mode` at creation
        // time (`hume-engine`'s `Pane` is the live SSOT — see
        // `commands::open_pane`). A same-buffer `:split`/`:vsplit` overrides
        // that seed with the source pane's live wrap mode instead (see
        // `commands::split_pane_onto`). Not per-buffer: wrap is a view
        // property, and two panes on the same buffer may wrap differently.
        // `scope` below additionally allows "pane" — `:set pane wrap-mode=…`
        // (see `typed_file::typed_set`) writes straight to the live `Pane`,
        // a separate path from `apply_setting`/this table.
        "wrap-mode" => wrap_mode: WrapMode = hume_engine::pane::DEFAULT_WRAP_STYLE,
            scope: ["global", "pane"],
            parser: from_str;
    }
    buffer {
        "tab-width" => tab_width: u8 = 4,
            scope: ["global", "buffer"],
            parser: tab_width;
        "tab-style" => tab_style: TabStyle = TabStyle::Hard,
            scope: ["global", "buffer"],
            parser: from_str;
        "line-number-style" => line_number_style: LineNumberStyle = LineNumberStyle::Hybrid,
            scope: ["global", "buffer"],
            parser: from_str;
        "auto-pairs-enabled" => auto_pairs_enabled: bool = true,
            scope: ["global", "buffer"],
            parser: bool;
    }
    extra_global {
        statusline: StatusLineConfig = StatusLineConfig::default();
        // Full whitespace config lives on EditorSettings; per-sub-field buffer
        // overrides live in BufferOverrides via override_only below.
        whitespace: WhitespaceConfig = WhitespaceConfig::default();
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
    }
    override_only {
        // Whitespace sub-fields are overridden independently so a buffer can
        // change just one (e.g. space) while still inheriting the global values
        // for the others (tab, newline). Resolution in BufferOverrides::whitespace.
        whitespace_space:   WhitespaceRender;
        whitespace_tab:     WhitespaceRender;
        whitespace_newline: WhitespaceRender;
    }
    manual_keys {
        // Sub-field patches (see apply_setting below) — not plain field writes.
        "whitespace-space"   => ["global", "buffer"];
        "whitespace-tab"     => ["global", "buffer"];
        "whitespace-newline" => ["global", "buffer"];
        // Parsed via parse_statusline, not FromStr — global-only.
        "statusline"         => ["global"];
    }
}

/// Parse the `"left|center|right"` wire format into a `StatusLineConfig`.
///
/// Requires exactly three `|`-separated sections. Each section is a
/// comma-separated list of `StatusElement` names; empty sections are allowed.
fn parse_statusline(s: &str) -> Result<StatusLineConfig, String> {
    let parts: Vec<&str> = s.splitn(4, '|').collect();
    if parts.len() != 3 {
        return Err(format!(
            "statusline value must be three sections separated by '|' \
             (e.g. 'Mode,FileName||Position'), got '{s}'"
        ));
    }
    let parse_section = |section: &str| -> Result<Vec<StatusElement>, String> {
        section
            .split(',')
            .filter(|name| !name.is_empty())
            .map(|name| name.parse::<StatusElement>())
            .collect()
    };
    Ok(StatusLineConfig {
        left: parse_section(parts[0])?,
        center: parse_section(parts[1])?,
        right: parse_section(parts[2])?,
    })
}

// ── BufferOverrides: manual accessors ─────────────────────────────────────────

impl BufferOverrides {
    /// Effective whitespace config, resolving each sub-field independently.
    ///
    /// Each of `space`, `tab`, and `newline` falls back to the global default
    /// when no buffer override is set for that sub-field. This lets a buffer
    /// override just one sub-field (e.g. `space`) while still inheriting the
    /// global values for the others.
    pub(crate) fn whitespace(&self, global: &EditorSettings) -> WhitespaceConfig {
        WhitespaceConfig {
            space: self.whitespace_space.unwrap_or(global.whitespace.space),
            tab: self.whitespace_tab.unwrap_or(global.whitespace.tab),
            newline: self.whitespace_newline.unwrap_or(global.whitespace.newline),
            // Rendering chars are not per-buffer configurable; always from global.
            ..global.whitespace.clone()
        }
    }

    /// Effective auto-pairs config for this buffer: `(enabled, &pairs)`.
    ///
    /// Returns references to avoid a `Vec` allocation on every keystroke.
    /// The `enabled` flag and the pair list are resolved independently so a
    /// buffer can override just one without replacing the other.
    pub(crate) fn auto_pairs_ref<'a>(&'a self, global: &'a EditorSettings) -> (bool, &'a [Pair]) {
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
        _ => Err(format!(
            "invalid value for '{key}': expected true/false, got '{value}'"
        )),
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
        assert!(s.pane_dividers);
    }

    #[test]
    fn buffer_overrides_default_is_all_none() {
        let ov = BufferOverrides::default();
        assert!(ov.tab_width.is_none());
        assert!(ov.tab_style.is_none());
        assert!(ov.line_number_style.is_none());
        assert!(ov.auto_pairs_enabled.is_none());
        assert!(ov.auto_pairs.is_none());
        assert!(ov.whitespace_space.is_none());
        assert!(ov.whitespace_tab.is_none());
        assert!(ov.whitespace_newline.is_none());
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
        assert_eq!(
            global("whitespace-newline", "all")
                .unwrap()
                .whitespace
                .newline,
            WhitespaceRender::All,
        );
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
        assert_eq!(ov.whitespace(&global).newline, WhitespaceRender::All);
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
        assert_eq!(ws.newline, WhitespaceRender::None); // inherited from global
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

    // ── all_setting_keys / setting_scopes / apply_setting cross-check ────────
    //
    // `manual_keys` unifies the macro-driven keys and the hand-listed extras
    // (whitespace-*/statusline) into one token stream, so `all_setting_keys()`
    // and `setting_scopes()` can no longer drift from *each other* by
    // construction. These guardrails catch the remaining drift risk: a key
    // added directly to `apply_setting`'s match without a corresponding
    // `manual_keys`/macro entry.

    #[test]
    fn all_setting_keys_have_declared_scopes() {
        for key in all_setting_keys() {
            assert!(
                !setting_scopes(key).is_empty(),
                "key '{key}' from all_setting_keys() has no declared scope in setting_scopes()"
            );
        }
    }

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
}
