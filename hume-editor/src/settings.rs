//! Centralized editor settings — the single source of truth for all
//! configurable editor behaviour.
//!
//! ## Layering
//!
//! ```text
//! hardcoded default → EditorSettings (global) → BufferOverrides (per-buffer)
//! ```
//!
//! [`EditorSettings`] holds concrete values for every setting; its [`Default`]
//! reproduces today's hardcoded defaults. [`BufferOverrides`] lives on each
//! [`crate::editor::buffer::Text`] and stores `Option<T>` per overridable
//! setting (`None` = inherit from global), resolved at call time via its
//! accessor methods — no pre-merged copy is kept.
//!
//! ## Adding a setting
//!
//! Most settings are defined in one [`define_settings!`] invocation that
//! generates [`EditorSettings`], [`BufferOverrides`], their `Default` impls,
//! accessors, and the [`write_setting`]/`setting_scopes` dispatch — a simple
//! setting needs one macro entry and nothing else. `scope: [...]` is the SSOT
//! for which `:set` scopes (`global`/`buffer`/`pane`) a key accepts;
//! `typed_set` looks it up via `setting_scopes(key)` rather than
//! special-casing per key. `"pane"` scope needs a matching `if key == "..."`
//! write arm in `typed_set` regardless of macro section placement, since
//! `write_setting` has no pane-storage concept.
//!
//! `language` has no macro entry: no global default (would let `:set global
//! language=…` silently succeed) and its write path needs `Editor`-level
//! access (`OnLanguageSet` hook, registry lookup) `write_setting`'s
//! `(&mut EditorSettings, &mut BufferOverrides)` signature doesn't have. It
//! stays a small, explicit special case in `typed_set`.
//!
//! Settings with non-trivial resolution (`auto_pairs_ref`, whitespace
//! sub-fields) are handled manually below the macro invocation.

use std::fmt;
use std::str::FromStr;

use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};

use crate::ops::auto_pairs::Pair;
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

// ── SignColumnConfig ──────────────────────────────────────────────────────────

/// Whether the sign column stays visible or collapses when empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignColumnMode {
    /// Always visible, regardless of whether any signs exist.
    #[default]
    Always,
    /// Collapses to zero width when no signs are visible in the current
    /// viewport (a sign elsewhere in the buffer, scrolled out of view,
    /// does not keep the column open).
    Auto,
}

impl fmt::Display for SignColumnMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Always => f.write_str("always"),
            Self::Auto => f.write_str("auto"),
        }
    }
}

impl FromStr for SignColumnMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "always" => Ok(Self::Always),
            "auto" => Ok(Self::Auto),
            _ => Err(format!(
                "invalid signcolumn mode: expected 'always' or 'auto', got '{s}'"
            )),
        }
    }
}

/// Sign column configuration: visibility mode and number of sign slots.
///
/// Wire format: `"always"`, `"always:N"`, `"auto"`, `"auto:N"` where N is the
/// number of sign slots (1–127). The gutter width is `columns + 1` (one cell
/// per sign plus one padding column). Default is `"always"` (= `"always:1"`,
/// width 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignColumnConfig {
    pub mode: SignColumnMode,
    /// Number of sign slots. Width = `columns + 1` (padding).
    pub columns: u8,
}

impl Default for SignColumnConfig {
    fn default() -> Self {
        Self {
            mode: SignColumnMode::Always,
            columns: 1,
        }
    }
}

impl SignColumnConfig {
    /// Gutter width in cells: one cell per sign slot plus one padding column.
    pub fn width(self) -> u8 {
        self.columns.saturating_add(1)
    }

    pub const VALUES: &'static [&'static str] = &["always", "auto", "always:1", "auto:1"];
}

impl fmt::Display for SignColumnConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.mode, self.columns)
    }
}

impl FromStr for SignColumnConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (mode_str, cols_str) = match s.split_once(':') {
            Some((m, c)) => (m, Some(c)),
            None => (s, None),
        };
        let mode: SignColumnMode = mode_str.parse()?;
        let columns = match cols_str {
            Some(c) => {
                let n: u8 = c.parse().map_err(|_| {
                    format!("invalid signcolumn columns: expected 1–127, got '{c}'")
                })?;
                if n == 0 || n > 127 {
                    return Err(format!(
                        "invalid signcolumn columns: expected 1–127, got '{n}'"
                    ));
                }
                n
            }
            None => 1,
        };
        Ok(Self { mode, columns })
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
/// All arms return `Result<T, String>`. Used inside `write_setting` (generated
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

/// Dispatch from a parser-kind token to the `get-option`-facing
/// [`hume_scripting::host::OptionValue`] shape. Mirrors [`parse_setting!`]'s
/// kind table so every setting stays readable the moment it's declared —
/// `bool` fields round-trip as `Bool`, integer-ish fields (`usize`,
/// `usize_nonzero`, `tab_width`) as `Int`, everything else (`from_str`,
/// `string`) via `Display`/`ToString` as `Str`. `from_str` types must
/// therefore implement `Display` that round-trips through their own
/// `FromStr` (see `TabStyle`, `DiagSeverity`, `LineNumberStyle`, `WrapMode`).
macro_rules! option_value {
    ($value:expr, bool) => {
        hume_scripting::host::OptionValue::Bool($value)
    };
    ($value:expr, usize) => {
        hume_scripting::host::OptionValue::Int($value as i64)
    };
    ($value:expr, usize_nonzero) => {
        hume_scripting::host::OptionValue::Int($value as i64)
    };
    ($value:expr, tab_width) => {
        hume_scripting::host::OptionValue::Int($value as i64)
    };
    ($value:expr, from_str) => {
        hume_scripting::host::OptionValue::Str($value.to_string())
    };
    ($value:expr, string) => {
        hume_scripting::host::OptionValue::Str($value)
    };
}

// ── Settings definition ───────────────────────────────────────────────────────

/// Generate [`EditorSettings`], [`BufferOverrides`], and [`write_setting`]
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
///   (not a plain field write) and so get a hand-written `write_setting` arm
///   below the macro invocation; format: `"key" => [scope, scope, ...];`.
///   Sole source for those keys' [`setting_scopes`]/[`all_setting_keys`]
///   entries — only the `write_setting` arm itself is hand-written.
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

        // ── write_setting ─────────────────────────────────────────────────────

        /// Write a setting's raw value — no derived-state resync.
        ///
        /// - `Global` scope writes to `settings` (always valid for all keys)
        /// - `Text` scope writes to `overrides` (rejected for global-only
        ///   keys)
        ///
        /// Returns `Err(message)` on unknown key, wrong-scope key, or invalid
        /// value.
        ///
        /// This is the raw field write only — some settings have derived
        /// state that must be resynced after a successful write (e.g. the
        /// undo-tree cap on every open buffer, the loaded theme). Production
        /// code must go through [`crate::editor::settings_ops::apply`], which
        /// wraps this and runs those effects; calling this directly would
        /// silently skip them.
        ///
        /// Stays `pub` (not `pub(crate)`) only because `testing/mock_host.rs`
        /// — which has no editor state to resync effects against, so it must
        /// call this raw writer — is `#[path]`-included into two external
        /// integration-test crates where `pub(crate)` would be invisible.
        /// `editor::lints::write_setting_only_called_from_allowlist` enforces
        /// the "chokepoint or MockHost only" restriction at the source level
        /// instead of via the type system.
        pub fn write_setting(
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
                    settings.whitespace.newline = parse_show_newline(value)?;
                }
                (SettingScope::Text, "whitespace-space") => {
                    overrides.whitespace_space = Some(value.parse()?);
                }
                (SettingScope::Text, "whitespace-tab") => {
                    overrides.whitespace_tab = Some(value.parse()?);
                }
                (SettingScope::Text, "whitespace-newline") => {
                    overrides.whitespace_newline = Some(parse_show_newline(value)?);
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

        // ── setting_value (get-option) ────────────────────────────────────────

        /// The effective value of `key` for `(get-option key)`: `overrides`'
        /// value if `Some` and the key is buffer-scoped, else the global
        /// default. `None` for a key with no generic storage — covers
        /// `manual_keys` (`whitespace-*`, `statusline`) and `"language"`,
        /// neither of which this getter supports today (no `core:lsp`
        /// feature reads them; add a hand-written arm here, mirroring
        /// `write_setting`'s manual arms, if one needs to).
        pub fn setting_value(
            key: &str,
            settings: &EditorSettings,
            overrides: Option<&BufferOverrides>,
        ) -> Option<hume_scripting::host::OptionValue> {
            match key {
                $( $gkey => Some(option_value!(settings.$gname.clone(), $gparser)), )*
                $( $bkey => {
                    let value = match overrides {
                        Some(o) => o.$bname(settings),
                        None => settings.$bname.clone(),
                    };
                    Some(option_value!(value, $bparser))
                } )*
                _ => None,
            }
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

        // ── is_bool_setting ───────────────────────────────────────────────────

        /// `true` if `key`'s value is parsed with `parser: bool` — i.e. its
        /// only valid values are `"true"`/`"false"`. Derived from the same
        /// per-key `parser: kind;` declaration used to dispatch parsing in
        /// `write_setting`, so a new bool setting is picked up automatically
        /// by anything that queries this (e.g.
        /// [`crate::editor::completion::SetCompleter`]'s value completion)
        /// instead of needing a hand-copied key list. `manual_keys` never
        /// declare a `parser:`, so this only checks global/buffer.
        pub(crate) fn is_bool_setting(key: &str) -> bool {
            match key {
                $( $gkey => stringify!($gparser) == "bool", )*
                $( $bkey => stringify!($bparser) == "bool", )*
                _ => false,
            }
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
        // 0 is a valid, meaningful value here (unlimited), unlike
        // history-capacity above — hence plain `usize`, not `usize_nonzero`.
        "undo-levels" => undo_levels: usize = 0,
            scope: ["global"],
            parser: usize;
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
        // rust-analyzer's first requests during indexing are slow — 10s
        // gives real-world servers room before the request is dropped as
        // TimedOut.
        "lsp.request-timeout-ms" => lsp_request_timeout_ms: usize = 10_000,
            scope: ["global"],
            parser: usize_nonzero;
        // Scroll bursts (page-down held, mouse wheel) must collapse to one
        // OnViewportChange fire, not one per frame.
        "lsp.viewport-debounce-ms" => lsp_viewport_debounce_ms: usize = 150,
            scope: ["global"],
            parser: usize;
        // Hint = most lenient — every severity renders. Gates the diagnostic
        // underline/extra-highlight and gutter-sign render write sides.
        "lsp.diagnostics-severity-floor" => lsp_diagnostics_severity_floor: crate::editor::lsp::DiagSeverity = crate::editor::lsp::DiagSeverity::Hint,
            scope: ["global"],
            parser: from_str;
        // Gates the inlay-hint render write side — off means the
        // `inlay_hints` store is untouched but nothing renders.
        "lsp.inlay-hints" => lsp_inlay_hints: bool = false,
            scope: ["global"],
            parser: bool;
        // Global-only *storage*: seeds new panes' `Pane::wrap_mode` at creation
        // time (`hume-engine`'s `Pane` is the live SSOT — see
        // `commands::open_pane`). A same-buffer `:split`/`:vsplit` overrides
        // that seed with the source pane's live wrap mode instead (see
        // `commands::split_pane_onto`). Not per-buffer: wrap is a view
        // property, and two panes on the same buffer may wrap differently.
        // `scope` below additionally allows "pane" — `:set pane wrap-mode=…`
        // (see `typed_file::typed_set`) writes straight to the live `Pane`,
        // a separate path from `write_setting`/this table.
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
        // After `c` (change), leaving Insert mode selects the text just
        // typed — see `cmd_change` and `end_insert_session`'s pinned-anchor
        // finalization.
        "select-changed-text" => select_changed_text: bool = true,
            scope: ["global", "buffer"],
            parser: bool;
        // Word motions (`w`/`W`/`b`/`B`) and `mm`/`MM` cover the destination
        // word's whitespace bookend (leading, or trailing for the first
        // word of a line) — see `word_select_cmd`/`run_native_body`'s
        // `around_fun` swap.
        "word-selects-whitespace" => word_selects_whitespace: bool = true,
            scope: ["global", "buffer"],
            parser: bool;
        "signcolumn" => signcolumn: SignColumnConfig = SignColumnConfig::default(),
            scope: ["global", "buffer"],
            parser: from_str;
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
        whitespace_newline: bool;
    }
    manual_keys {
        // Sub-field patches (see write_setting below) — not plain field writes.
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

/// The wire-format strings [`parse_show_newline`] accepts — the single
/// source `:set buffer whitespace-newline=<Tab>` completion mirrors (see
/// `editor::completion::set::static_value_candidates`), so the two can never
/// drift out of sync. Mirrors the `WhitespaceRender::VALUES` pattern
/// (`hume-engine/src/pane.rs`) used by the sibling `space`/`tab` settings.
pub(crate) const SHOW_NEWLINE_VALUES: &[&str] = &["none", "all"];

/// Parse the `whitespace-newline` wire format. Unlike `space`/`tab`, a
/// newline is inherently always at end-of-line, so there's no meaningful
/// "trailing" distinction — only `none`/`all`.
fn parse_show_newline(s: &str) -> Result<bool, String> {
    match s.to_ascii_lowercase().as_str() {
        "none" => Ok(false),
        "all" => Ok(true),
        _ => Err(format!(
            "invalid whitespace-newline '{s}': expected none or all"
        )),
    }
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
mod tests;
