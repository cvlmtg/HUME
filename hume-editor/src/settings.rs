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
//! accessors, and the [`write_global`]/[`write_buffer`]/`setting_scopes`
//! dispatch — a simple setting needs one macro entry and nothing else.
//! `scope: [...]` is the SSOT for which [`Scope`] variants a key accepts;
//! `typed_set` looks it up via `setting_scopes(key)` rather than
//! special-casing per key. `Scope::Pane` needs a matching `if key == "..."`
//! write arm in `typed_set` regardless of macro section placement, since
//! neither `write_global` nor `write_buffer` has a pane-storage concept.
//!
//! A global entry that has a derived-state effect beyond the raw field write
//! (resizing a live ring, reloading a resource) declares `resync: true` —
//! see `editor::settings_ops::apply_global`'s doc for how that's wired to the
//! actual effect and enforced against drift.
//!
//! `language` has no macro entry: no global default (would let `:set global
//! language=…` silently succeed) and its write path needs `Editor`-level
//! access (`OnLanguageSet` hook, registry lookup) that `write_buffer`'s
//! `(&mut BufferOverrides)` signature doesn't have. It stays a small,
//! explicit special case in `typed_set`.
//!
//! Settings with non-trivial resolution (`auto_pairs_ref`) are handled
//! manually below the macro invocation. Sub-fields of a larger config value
//! (the three `whitespace-*` keys) use the `subfield` section instead of
//! `global`/`buffer` — see its doc below.

use std::fmt;
use std::str::FromStr;

use hume_editing::tab_style::TabStyle;
use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};

use crate::ui::statusline::{StatusElement, StatusLineConfig};
use hume_ops::auto_pairs::Pair;

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

// ── Scope ─────────────────────────────────────────────────────────────────────

/// A `:set` scope token: `global`, `buffer`, or `pane`.
///
/// `Global` applies to editor-wide defaults (written to [`EditorSettings`] via
/// [`write_global`]). `Buffer` overrides a setting for the active buffer only
/// (written to [`BufferOverrides`] via [`write_buffer`]). `Pane` has no
/// generic storage at all — the sole pane-scoped key (`wrap-mode`) writes
/// straight to the live `Pane` in `typed_file::typed_set`, bypassing both of
/// the above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Buffer,
    Pane,
}

impl Scope {
    /// Every scope, in the order `:set`'s scope-phase completion offers them
    /// (alphabetical, applied by the caller).
    pub const ALL: &'static [Scope] = &[Scope::Global, Scope::Buffer, Scope::Pane];

    /// The wire-format string for this scope — the single source `Display`
    /// delegates to and completion/error messages format with, so the two
    /// can never drift out of sync.
    pub const fn as_str(self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Buffer => "buffer",
            Scope::Pane => "pane",
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Scope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "global" => Ok(Scope::Global),
            "buffer" => Ok(Scope::Buffer),
            "pane" => Ok(Scope::Pane),
            _ => Err(format!("unknown :set scope '{s}'")),
        }
    }
}

/// The `:set`/completion key for a buffer's language identity. Not a
/// `define_settings!` entry (see the module doc's "Adding a setting"
/// section for why) — this constant is the single source `typed_set` and
/// `completion::set` compare against, so the two special cases can't drift
/// on the literal.
pub(crate) const LANGUAGE_KEY: &str = "language";

// ── Parser helper ─────────────────────────────────────────────────────────────

/// Dispatch from a parser-kind token to the actual parse call.
///
/// All arms return `Result<T, String>`. Used inside `write_global`/
/// `write_buffer` (generated by `define_settings!`) where `value` and `key`
/// are in scope.
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
    ($value:expr, $key:expr, show_newline) => {
        parse_show_newline($value)
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
/// `show_newline` stores a plain `bool` but its wire format is `none`/`all`,
/// so it round-trips as `Str` via [`format_show_newline`], the inverse of
/// [`parse_show_newline`].
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
    ($value:expr, show_newline) => {
        hume_scripting::host::OptionValue::Str(format_show_newline($value).to_string())
    };
}

// ── Settings definition ───────────────────────────────────────────────────────

/// Generate [`EditorSettings`], [`BufferOverrides`], [`write_global`], and
/// [`write_buffer`] from a single source of truth.
///
/// ## Sections
///
/// - `global { … }` — global-only settings with a `:set` key; format:
///   `"key" => field: Type = default, scope: [...], parser: kind [, resync: true];`
///   `resync: true` is optional — declare it when writing this key has a
///   derived-state effect beyond the raw field write (see
///   `editor::settings_ops::apply_global`'s doc).
/// - `buffer { … }` — per-buffer-overridable settings with a `:set` key;
///   same format (no `resync:` — no buffer-scoped key needs one today; add
///   the clause to this section's grammar first if one ever does)
/// - `extra_global { … }` — extra fields on `EditorSettings` only, no `:set`
///   key; format: `field: Type = default;`
/// - `extra_buffer { … }` — extra fields on both structs, no `:set` key;
///   format: `field: Type = global_default;` (buffer default is always `None`)
/// - `subfield { … }` — a `:set` key whose global storage is a nested field
///   of an `extra_global` value (not a top-level `EditorSettings` field) and
///   whose buffer override has a different field name than its global path
///   (so it can't reuse the `buffer` section's "one name, two structs"
///   shape); format:
///   `"key" => global_field.sub_field / override_field : Type, scope: [...], parser: kind;`
///   Generates the `BufferOverrides` field, both write arms, and the
///   `setting_value` arm. The three `whitespace-*` keys are the only users.
/// - `manual_keys { … }` — `:set` keys whose values need custom resolution
///   (not a plain field write) and so get a hand-written write arm below the
///   macro invocation; format: `"key" => [scope, ...];`. Sole source for
///   those keys' [`setting_scopes`]/[`all_setting_keys`] entries — only the
///   write arms themselves are hand-written. `"statusline"` is the only user
///   (three `|`-separated sections, not a single value).
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
/// | `string` | `value.to_owned()` |
/// | `show_newline` | `parse_show_newline(value)` (`none`/`all` wire format) |
macro_rules! define_settings {
    (
        global {
            $( $gkey:literal => $gname:ident : $gtype:ty = $gdefault:expr, scope: [$($gscope:expr),+], parser: $gparser:ident $(, resync: $gresync:literal)?; )*
        }
        buffer {
            $( $bkey:literal => $bname:ident : $btype:ty = $bdefault:expr, scope: [$($bscope:expr),+], parser: $bparser:ident; )*
        }
        extra_global {
            $( $egname:ident : $egtype:ty = $egdefault:expr; )*
        }
        extra_buffer {
            $( $ebname:ident : $ebtype:ty = $ebdefault:expr; )*
        }
        subfield {
            $( $skey:literal => $sglobal:ident . $ssub:ident / $sfield:ident : $stype:ty, scope: [$($sscope:expr),+], parser: $sparser:ident; )*
        }
        manual_keys {
            $( $mkey:literal => [$($mscope:expr),+]; )*
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
            $( pub $sfield: Option<$stype>, )*
        }

        impl BufferOverrides {
            $(
                /// Effective value: buffer override → global default.
                pub(crate) fn $bname(&self, global: &EditorSettings) -> $btype {
                    self.$bname.clone().unwrap_or_else(|| global.$bname.clone())
                }
            )*
        }

        // ── write_global / write_buffer ───────────────────────────────────────

        /// Write a global setting's raw value — no derived-state resync.
        ///
        /// Returns `Err(message)` on unknown key or invalid value.
        ///
        /// This is the raw field write only — some settings have derived
        /// state that must be resynced after a successful write (declared
        /// via `resync: true` above). Production code must go through
        /// [`crate::editor::settings_ops::apply_global`], which wraps this
        /// and runs those effects; calling this directly would silently skip
        /// them.
        ///
        /// Stays `pub` (not `pub(crate)`) only because `testing/mock_host.rs`
        /// — which has no editor state to resync effects against, so it must
        /// call this raw writer — is `#[path]`-included into two external
        /// integration-test crates where `pub(crate)` would be invisible.
        /// `editor::lints::write_global_and_write_buffer_only_called_from_allowlist`
        /// enforces the "chokepoint or MockHost only" restriction at the
        /// source level instead of via the type system.
        pub fn write_global(key: &str, value: &str, settings: &mut EditorSettings) -> Result<(), String> {
            match key {
                $( $gkey => { settings.$gname = parse_setting!(value, key, $gparser)?; } )*
                $( $bkey => { settings.$bname = parse_setting!(value, key, $bparser)?; } )*
                $( $skey => { settings.$sglobal.$ssub = parse_setting!(value, key, $sparser)?; } )*
                // Statusline config — global-only; three sections separated by `|`,
                // each a comma-separated list of StatusElement names (may be empty).
                "statusline" => { settings.statusline = parse_statusline(value)?; }
                _ => return Err(format!("unknown setting '{key}'")),
            }
            Ok(())
        }

        /// Write a buffer-scoped setting's raw override — no derived-state
        /// resync (no buffer-scoped key has one today; see [`write_global`]'s
        /// doc for the mechanism global-only keys use).
        ///
        /// Returns `Err(message)` on unknown key, a global-only key, or an
        /// invalid value.
        pub fn write_buffer(key: &str, value: &str, overrides: &mut BufferOverrides) -> Result<(), String> {
            match key {
                $( $bkey => { overrides.$bname = Some(parse_setting!(value, key, $bparser)?); } )*
                $( $skey => { overrides.$sfield = Some(parse_setting!(value, key, $sparser)?); } )*
                "statusline" => {
                    return Err("'statusline' is a global-only setting — use :set global statusline=…".to_string());
                }
                $( $gkey => {
                    return Err(format!(
                        "'{key}' is a global-only setting — use :set global {key}=…"
                    ));
                } )*
                _ => return Err(format!("unknown setting '{key}'")),
            }
            Ok(())
        }

        // ── has_declared_resync ────────────────────────────────────────────────

        /// `true` if `key` declared `resync: true` on its `global {}` entry —
        /// the SSOT `editor::settings_ops::resync_derived_state`'s fallthrough
        /// arm checks against via `debug_assert!`, so a key that declares a
        /// resync effect but has no matching arm there fails loudly instead
        /// of silently doing nothing.
        pub(crate) fn has_declared_resync(key: &str) -> bool {
            match key {
                $( $( $gkey => $gresync, )? )*
                _ => false,
            }
        }

        // ── setting_value (get-option) ────────────────────────────────────────

        /// The effective value of `key` for `(get-option key)`: `overrides`'
        /// value if `Some` and the key is buffer-scoped, else the global
        /// default. `None` for a key with no generic storage — covers only
        /// `"language"` today, which has no getter (it lives on the
        /// buffer's language identity — use `(buffer-language bid)`
        /// instead).
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
                $( $skey => {
                    let value = match overrides {
                        Some(o) => o.$sfield.unwrap_or(settings.$sglobal.$ssub),
                        None => settings.$sglobal.$ssub,
                    };
                    Some(option_value!(value, $sparser))
                } )*
                "statusline" => Some(hume_scripting::host::OptionValue::Str(
                    format_statusline(&settings.statusline),
                )),
                _ => None,
            }
        }

        // ── setting_scopes ──────────────────────────────────────────────────────

        /// The [`Scope`]s a setting accepts, as declared by its `scope: [...]`
        /// list in the [`define_settings!`] invocation below. Empty for any
        /// key not declared there — notably `"language"`, which has no
        /// generic storage and is handled entirely by `typed_set`'s own
        /// special case, never through this table.
        pub(crate) fn setting_scopes(key: &str) -> &'static [Scope] {
            match key {
                $( $gkey => &[$($gscope),+], )*
                $( $bkey => &[$($bscope),+], )*
                $( $skey => &[$($sscope),+], )*
                $( $mkey => &[$($mscope),+], )*
                _ => &[],
            }
        }

        // ── all_setting_keys ───────────────────────────────────────────────────

        /// Every setting key with a `:set` wire format — the union of the
        /// `global`/`buffer`/`subfield` macro entries and the `manual_keys`
        /// entries (`statusline`). Notably **excludes** `"language"`, which
        /// has no macro entry and is surfaced only when the completer knows
        /// the scope is `"buffer"` (its sole valid scope). Used by
        /// [`crate::editor::completion::SetCompleter`] to enumerate key
        /// candidates, filtered further by [`setting_scopes`] against the
        /// chosen scope.
        pub(crate) fn all_setting_keys() -> &'static [&'static str] {
            &[$($gkey,)* $($bkey,)* $($skey,)* $($mkey,)*]
        }

        // ── is_bool_setting ───────────────────────────────────────────────────

        /// `true` if `key`'s value is parsed with `parser: bool` — i.e. its
        /// only valid values are `"true"`/`"false"`. Derived from the same
        /// per-key `parser: kind;` declaration used to dispatch parsing in
        /// `write_global`/`write_buffer`, so a new bool setting is picked up
        /// automatically by anything that queries this (e.g.
        /// [`crate::editor::completion::SetCompleter`]'s value completion)
        /// instead of needing a hand-copied key list. `manual_keys` never
        /// declare a `parser:`, so this only checks global/buffer/subfield.
        pub(crate) fn is_bool_setting(key: &str) -> bool {
            match key {
                $( $gkey => stringify!($gparser) == "bool", )*
                $( $bkey => stringify!($bparser) == "bool", )*
                $( $skey => stringify!($sparser) == "bool", )*
                _ => false,
            }
        }
    };
}

define_settings! {
    global {
        "scrolloff" => scrolloff: usize = 3,
            scope: [Scope::Global],
            parser: usize;
        "mouse-scroll-lines" => mouse_scroll_lines: usize = 3,
            scope: [Scope::Global],
            parser: usize;
        "mouse-enabled" => mouse_enabled: bool = true,
            scope: [Scope::Global],
            parser: bool;
        "mouse-select" => mouse_select: bool = false,
            scope: [Scope::Global],
            parser: bool;
        // Resizes every open pane's live jump list cap — like undo-levels
        // below, takes effect on the next push, not retroactively. See
        // `editor::settings_ops::resync_derived_state` and `JumpList::set_capacity`.
        "jump-list-capacity" => jump_list_capacity: usize = 100,
            scope: [Scope::Global],
            parser: usize_nonzero,
            resync: true;
        "jump-line-threshold" => jump_line_threshold: usize = 5,
            scope: [Scope::Global],
            parser: usize;
        // Resizes the command/search prompt-history ring cap — like
        // undo-levels below, takes effect on the next push, not
        // retroactively. See `editor::settings_ops::resync_derived_state`
        // and `History::set_capacity`.
        "history-capacity" => history_capacity: usize = 100,
            scope: [Scope::Global],
            parser: usize_nonzero,
            resync: true;
        // 0 is a valid, meaningful value here (unlimited), unlike
        // history-capacity above — hence plain `usize`, not `usize_nonzero`.
        // Resizes the undo-tree cap on every open buffer — takes effect on
        // the next edit, not retroactively (Vim's `undolevels` semantics).
        // See `editor::settings_ops::resync_derived_state`.
        "undo-levels" => undo_levels: usize = 0,
            scope: [Scope::Global],
            parser: usize,
            resync: true;
        "steel-init-budget-ms" => steel_init_budget_ms: usize = 10_000,
            scope: [Scope::Global],
            parser: usize_nonzero;
        "steel-command-budget-ms" => steel_command_budget_ms: usize = 1_000,
            scope: [Scope::Global],
            parser: usize_nonzero;
        "popup-border" => popup_border: bool = true,
            scope: [Scope::Global],
            parser: bool;
        "pane-dividers" => pane_dividers: bool = true,
            scope: [Scope::Global],
            parser: bool;
        // Read fresh by the statusline provider each frame — no resync needed.
        "statusline.mode-colors" => statusline_mode_colors: bool = true,
            scope: [Scope::Global],
            parser: bool;
        // Loads and applies the named theme immediately, rolling back to the
        // previous value on failure — see
        // `editor::settings_ops::resync_derived_state`.
        "theme" => theme: String = String::new(),
            scope: [Scope::Global],
            parser: string,
            resync: true;
        "syntax-highlight-max-bytes" => syntax_highlight_max_bytes: usize = 1_048_576,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // rust-analyzer's first requests during indexing are slow — 10s
        // gives real-world servers room before the request is dropped as
        // TimedOut.
        "lsp.request-timeout-ms" => lsp_request_timeout_ms: usize = 10_000,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // Scroll bursts (page-down held, mouse wheel) must collapse to one
        // OnViewportChange fire, not one per frame.
        "lsp.viewport-debounce-ms" => lsp_viewport_debounce_ms: usize = 150,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // Hint = most lenient — every severity renders. Gates the diagnostic
        // underline/extra-highlight and gutter-sign render write sides.
        "lsp.diagnostics-severity-floor" => lsp_diagnostics_severity_floor: crate::editor::lsp::diagnostics::DiagSeverity = crate::editor::lsp::diagnostics::DiagSeverity::Hint,
            scope: [Scope::Global],
            parser: from_str;
        // Gates the inlay-hint render write side — off means the
        // `inlay_hints` store is untouched but nothing renders.
        "lsp.inlay-hints" => lsp_inlay_hints: bool = false,
            scope: [Scope::Global],
            parser: bool;
        // Global-only *storage*: seeds new panes' `Pane::wrap_mode` at creation
        // time (`hume-engine`'s `Pane` is the live SSOT — see
        // `commands::open_pane`). A same-buffer `:split`/`:vsplit` overrides
        // that seed with the source pane's live wrap mode instead (see
        // `commands::split_pane_onto`). Not per-buffer: wrap is a view
        // property, and two panes on the same buffer may wrap differently.
        // `scope` below additionally allows `Scope::Pane` — `:set pane
        // wrap-mode=…` (see `typed_file::typed_set`) writes straight to the
        // live `Pane`, a separate path from `write_global`/this table.
        "wrap-mode" => wrap_mode: WrapMode = hume_engine::pane::DEFAULT_WRAP_STYLE,
            scope: [Scope::Global, Scope::Pane],
            parser: from_str;
    }
    buffer {
        "tab-width" => tab_width: u8 = 4,
            scope: [Scope::Global, Scope::Buffer],
            parser: tab_width;
        "indent-guides" => show_indent_guides: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        "tab-style" => tab_style: TabStyle = TabStyle::Hard,
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        "line-number-style" => line_number_style: LineNumberStyle = LineNumberStyle::Hybrid,
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        "auto-pairs-enabled" => auto_pairs_enabled: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        // After `c` (change), leaving Insert mode selects the text just
        // typed — see `cmd_change` and `end_insert_session`'s pinned-anchor
        // finalization.
        "select-changed-text" => select_changed_text: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        // Word motions (`w`/`W`/`b`/`B`) and `mm`/`MM` cover the destination
        // word's whitespace bookend (leading, or trailing for the first
        // word of a line) — see `word_select_cmd`/`run_native_body`'s
        // `around_fun` swap.
        "word-selects-whitespace" => word_selects_whitespace: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        "signcolumn" => signcolumn: SignColumnConfig = SignColumnConfig::default(),
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        // Read fresh by `check_buffer_disk_state` at each trigger — no
        // resync needed. `true`: an external change to the focused buffer
        // opens a reload confirm. `false`: detection still runs and warns,
        // but reload stays manual via `:e!`/`:checktime`. Independent of
        // `:w`'s write guard, which stats the file itself at write time
        // regardless of this setting — see `stale_write_block`.
        "autoread" => autoread: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
    }
    extra_global {
        statusline: StatusLineConfig = StatusLineConfig::default();
        // Full whitespace config lives on EditorSettings; per-sub-field buffer
        // overrides are declared in `subfield` below.
        whitespace: WhitespaceConfig = WhitespaceConfig::default();
    }
    extra_buffer {}
    subfield {
        // Whitespace sub-fields are overridden independently so a buffer can
        // change just one (e.g. space) while still inheriting the global
        // values for the others. Resolution in `BufferOverrides::whitespace`.
        "whitespace-space" => whitespace.space / whitespace_space : WhitespaceRender,
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        "whitespace-tab" => whitespace.tab / whitespace_tab : WhitespaceRender,
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        "whitespace-newline" => whitespace.newline / whitespace_newline : bool,
            scope: [Scope::Global, Scope::Buffer],
            parser: show_newline;
    }
    manual_keys {
        // Parsed via parse_statusline, not FromStr — global-only.
        "statusline" => [Scope::Global];
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

/// Render a `StatusLineConfig` back to the `"left|center|right"` wire format
/// [`parse_statusline`] accepts — the inverse, used by `(get-option
/// "statusline")` and by `configure-statusline!`'s re-serialization before
/// handing the value to the `write_global` chokepoint.
pub(crate) fn format_statusline(cfg: &StatusLineConfig) -> String {
    let join = |elems: &[StatusElement]| {
        elems
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{}|{}|{}",
        join(&cfg.left),
        join(&cfg.center),
        join(&cfg.right)
    )
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

/// Render a `whitespace-newline` value back to the wire format
/// [`parse_show_newline`] accepts — the inverse, used by
/// `(get-option "whitespace-newline")`.
fn format_show_newline(value: bool) -> &'static str {
    if value { "all" } else { "none" }
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
            ..global.whitespace
        }
    }

    /// Effective auto-pairs config for this buffer: `(enabled, &pairs)`.
    ///
    /// The pair list itself is a fixed constant (`hume_ops::auto_pairs::DEFAULT_PAIRS`)
    /// — only `auto-pairs-enabled` is an actual per-buffer setting.
    pub(crate) fn auto_pairs_ref(&self, global: &EditorSettings) -> (bool, &'static [Pair]) {
        (
            self.auto_pairs_enabled(global),
            hume_ops::auto_pairs::DEFAULT_PAIRS,
        )
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
