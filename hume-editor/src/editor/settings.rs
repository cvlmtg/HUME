//! Editor settings: the single source of truth for configurable behaviour.
//!
//! [`EditorSettings`] holds a concrete value for every setting.
//! [`BufferOverrides`] lives on each `Buffer` with an `Option<T>` per
//! overridable setting (`None` inherits the global), resolved at call time by
//! its accessors; no merged copy is kept. Each also records which
//! `ConfigOrigin` wrote each value it holds, and a write ranks against that
//! record ([`SettingOrigins`]).
//!
//! ## Adding a setting
//!
//! A simple setting is one entry in the `define_settings!` invocation, which
//! generates both structs, their defaults, accessors and the
//! `write_global`/`write_buffer`/`setting_scopes` dispatch. `scope: [...]` is
//! the SSOT for accepted scopes, but `Scope::Pane` still needs a write arm in
//! `typed_set`, since neither generated writer has pane storage. A global key
//! whose write has a derived-state effect declares `resync: true` (wired in
//! `editor::settings::ops::apply_global`).
//!
//! The `BufferIntrinsicOption`s have no macro entry: they have no global
//! default and live on the buffer. `language` needs `Editor`-level access
//! (`OnLanguageSet` hook, registry lookup), so `typed_set` handles it;
//! `line-ending` is written by `ops::set_line_ending` and read by the host's
//! `buffer_line_ending`.

use std::fmt;
use std::str::FromStr;

use hume_core::{Claim, ConfigOrigin, DiagSeverity, LaterWins, OptionInput, StatuslineSections};
use hume_editing::tab_style::TabStyle;
use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};
use hume_platform::process::batch::ShellCommand;
use rustc_hash::FxHashMap;

use crate::statusline::{StatusElement, StatusLineConfig};
use hume_ops::auto_pairs::Pair;

// ── settings_enum! ────────────────────────────────────────────────────────────

/// `"a"`, `"a or b"`, `"a, b, or c"`: how [`parse_named`]'s error lists the
/// values it would have accepted.
fn or_list(values: &[&str]) -> String {
    match values {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [a, b] => format!("{a} or {b}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

/// The value whose name in `names` matches `s` ignoring ASCII case, `values`
/// being in `names`' order: the parse of every closed `:set` value. The error
/// names every accepted value.
fn parse_named<T>(
    s: &str,
    names: &[&str],
    values: impl IntoIterator<Item = T>,
) -> Result<T, String> {
    names
        .iter()
        .zip(values)
        .find(|(name, _)| name.eq_ignore_ascii_case(s))
        .map(|(_, value)| value)
        .ok_or_else(|| format!("expected {}, got '{s}'", or_list(names)))
}

/// The text form of a `:set` value: the names `:set <key>=<Tab>` completes,
/// the parser that reads a value, and the printer that writes it back.
///
/// Every value type behind a `from_str` setting implements it,
/// whichever crate defines the type. A trait local to this crate is the only
/// way to give a foreign type that surface, which is why the text forms of
/// the option enums live here and not next to the enums.
pub(in crate::editor) trait SettingText: Sized {
    /// The strings [`SettingText::parse`] accepts, offered by completion.
    const VALUES: &'static [&'static str];

    fn parse(s: &str) -> Result<Self, String>;

    /// The text [`SettingText::parse`] reads back as `self`.
    fn to_text(&self) -> String;
}

/// The origin of the write behind each value a settings store holds, by
/// key. A key with none holds its compiled-in default (global) or no
/// override (buffer). Within one scope a write from an origin below the
/// recorded one is shadowed, and one from the same or a higher origin
/// replaces it.
#[derive(Clone, Debug, Default)]
pub(in crate::editor::settings) struct SettingOrigins(FxHashMap<&'static str, Claim<LaterWins>>);

impl SettingOrigins {
    /// Whether `by` may write `key`; recorded as its writer when it may.
    fn claim(&mut self, key: &'static str, by: ConfigOrigin) -> bool {
        self.0.entry(key).or_default().claim(by).is_applied()
    }

    /// `key`'s slot as it stands, for [`Self::restore`].
    pub(in crate::editor::settings) fn slot(&self, key: &str) -> Claim<LaterWins> {
        self.0.get(key).copied().unwrap_or_default()
    }

    /// Put back what [`Self::slot`] read for `key` before a write that is
    /// being undone.
    pub(in crate::editor::settings) fn restore(
        &mut self,
        key: &'static str,
        slot: Claim<LaterWins>,
    ) {
        self.0.insert(key, slot);
    }
}

/// Name the setting `key` in a parse error. Every `:set` value parse goes
/// through here, so the parsers themselves stay key-free.
pub(in crate::editor) fn for_key<T>(key: &str, parsed: Result<T, String>) -> Result<T, String> {
    parsed.map_err(|e| format!("invalid value for '{key}': {e}"))
}

/// The text of `value` for a setting that takes one value; a list or the
/// statusline's sections are refused.
pub(in crate::editor) fn single_value(value: &OptionInput) -> Result<&str, String> {
    match value {
        OptionInput::Text(text) => Ok(text),
        OptionInput::Words(_) => Err("takes one value, not a list".to_string()),
        OptionInput::Sections(_) => Err("takes one value, not statusline sections".to_string()),
    }
}

/// Parse `value` as the setting `key`'s type.
pub(in crate::editor) fn parse_setting_text<T: SettingText>(
    key: &str,
    value: &str,
) -> Result<T, String> {
    for_key(key, T::parse(value))
}

/// Generate a closed `:set` enum's [`SettingText`] from one list of
/// `Variant => "name"` pairs. The parse is case-insensitive and its error
/// names every accepted value.
///
/// Only the enum declaration stays hand-written, so each variant keeps its
/// own doc comment. `VALUES`, the parsed values and the print arms all come
/// from the one list, so a variant cannot be completable but unparseable.
macro_rules! settings_enum {
    ($ty:ty, [$($variant:ident => $name:literal),+ $(,)?]) => {
        impl SettingText for $ty {
            const VALUES: &'static [&'static str] = &[$($name),+];

            fn parse(s: &str) -> Result<Self, String> {
                parse_named(s, Self::VALUES, [$(Self::$variant),+])
            }

            fn to_text(&self) -> String {
                match self {
                    $(Self::$variant => $name,)+
                }
                .to_owned()
            }
        }
    };
}

/// A `:set` scope token: `global`, `buffer`, or `pane`.
///
/// `Global` applies to editor-wide defaults (written to [`EditorSettings`] via
/// [`write_global`]). `Buffer` overrides a setting for the active buffer only
/// (written to [`BufferOverrides`] via [`write_buffer`]). `Pane` has no
/// generic storage at all: the sole pane-scoped key (`wrap-mode`) writes
/// straight to the live `Pane` in `typed_file::typed_set`, a third, narrower
/// rung *on top of* `Buffer`/`Global` rather than a bypass of them: a pane
/// with no pane-level override still resolves through the buffer/global
/// chain (see `commands::effective_wrap_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum Scope {
    Global,
    Buffer,
    Pane,
}

impl Scope {
    /// Every scope, in the order `:set`'s scope-phase completion offers them
    /// (alphabetical, applied by the caller).
    pub(in crate::editor) const ALL: &'static [Scope] =
        &[Scope::Global, Scope::Buffer, Scope::Pane];

    /// The wire-format string for this scope: the single source `Display`
    /// delegates to and completion/error messages format with, so the two
    /// can never drift out of sync.
    pub(in crate::editor) const fn as_str(self) -> &'static str {
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

/// The `:set`/completion key for the active theme, declared as a
/// `define_settings!` entry (below), but also matched directly at a few
/// non-macro call sites, so this constant keeps those literals from drifting
/// off the macro's own key string.
pub(in crate::editor) const THEME_KEY: &str = "theme";

/// Name the `theme` setting starts as: the theme loaded from the search path
/// when config doesn't pick one.
pub(in crate::editor) const DEFAULT_THEME: &str = "sand";

/// Same rationale as [`THEME_KEY`], for `wrap-mode`'s non-macro call sites.
pub(in crate::editor) const WRAP_MODE_KEY: &str = "wrap-mode";

/// One global setting key declared `resync: true` in `define_settings!`.
///
/// Only the generated `resync_key` produces values, so drift is a compile
/// error both ways: a new `resync: true` key with no variant fails at
/// `resync_key`, and a variant with no arm fails `resync_derived_state`'s
/// exhaustive `match`. Variants reuse the snake_case field names because
/// `resync_key` emits `$gname` verbatim (renaming would need `paste`).
///
/// Omit the clause rather than writing `resync: false`: that still compiles,
/// but produces a variant and an arm that can never fire.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(non_camel_case_types)]
pub(in crate::editor) enum ResyncKey {
    jump_list_capacity,
    prompt_history_levels,
    undo_levels,
    theme,
}

// ── Parser helper ─────────────────────────────────────────────────────────────

/// Dispatch from a parser-kind token to the actual parse call.
///
/// All arms return `Result<T, String>`. Used inside `write_global`/
/// `write_buffer` (generated by `define_settings!`) where `value` and `key`
/// are in scope.
macro_rules! parse_setting {
    (@raw $value:expr, bool) => {
        parse_bool($value)
    };
    (@raw $value:expr, usize) => {
        parse_usize($value)
    };
    (@raw $value:expr, usize_nonzero) => {
        parse_usize_nonzero($value)
    };
    (@raw $value:expr, tab_width) => {
        parse_tab_width($value)
    };
    (@raw $value:expr, from_str) => {
        SettingText::parse($value)
    };
    (@raw $value:expr, string) => {
        Ok::<String, String>(($value).to_owned())
    };
    (@raw $value:expr, show_newline) => {
        parse_show_newline($value)
    };
    (@raw $value:expr, word_chars) => {
        parse_word_chars($value)
    };
    ($value:expr, $key:expr, shell_command) => {
        for_key(
            $key,
            match $value {
                OptionInput::Text(text) => ShellCommand::parse(text),
                OptionInput::Words(words) => ShellCommand::from_words(words.clone()),
                OptionInput::Sections(_) => Err("takes a command, not statusline sections".to_string()),
            },
        )
    };
    ($value:expr, $key:expr, $kind:ident) => {
        for_key(
            $key,
            single_value($value).and_then(|text| parse_setting!(@raw text, $kind)),
        )
    };
}

/// Dispatch from a parser-kind token to the `get-option`-facing
/// [`hume_scripting::host::OptionValue`] shape. Mirrors [`parse_setting!`]'s
/// kind table so every setting stays readable the moment it's declared:
/// `bool` fields read back as `Bool`, integer-ish fields (`usize`,
/// `usize_nonzero`, `tab_width`) as `Int`, and every named value as `Str`
/// in the text its own parser takes back: [`SettingText::to_text`] for
/// `from_str` types, [`format_show_newline`] (the inverse of
/// [`parse_show_newline`]) for `show_newline`. `shell_command` reads back as
/// `List`, one string per word.
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
        hume_scripting::host::OptionValue::Str($value.to_text())
    };
    ($value:expr, string) => {
        hume_scripting::host::OptionValue::Str($value)
    };
    ($value:expr, show_newline) => {
        hume_scripting::host::OptionValue::Str(format_show_newline($value).to_string())
    };
    ($value:expr, word_chars) => {
        hume_scripting::host::OptionValue::Str($value.to_string())
    };
    ($value:expr, shell_command) => {
        hume_scripting::host::OptionValue::List($value.words())
    };
}

/// Dispatch from a parser-kind token to a `BufferOverrides` accessor.
///
/// Every buffer setting but `word-chars` is `Copy`, so cloning it to resolve
/// buffer-override-or-global is free, and the default arm does that. `word_chars`
/// is the one `String`-typed buffer setting; borrowing instead of cloning is
/// what lets [`hume_editing::word::WordChars`] stay borrowed-and-`Copy` per
/// its own doc, rather than every caller paying a heap clone per keystroke.
macro_rules! buffer_accessor {
    ($bname:ident, $btype:ty, word_chars) => {
        /// Effective value: buffer override → global default.
        pub(in crate::editor) fn $bname<'a>(&'a self, global: &'a EditorSettings) -> &'a str {
            self.$bname.as_deref().unwrap_or(&global.$bname)
        }
    };
    ($bname:ident, $btype:ty, $bparser:ident) => {
        /// Effective value: buffer override → global default.
        pub(in crate::editor) fn $bname(&self, global: &EditorSettings) -> $btype {
            self.$bname.clone().unwrap_or_else(|| global.$bname.clone())
        }
    };
}

// ── Settings definition ───────────────────────────────────────────────────────

/// Generate [`EditorSettings`], [`BufferOverrides`], [`write_global`], and
/// [`write_buffer`] from a single source of truth.
///
/// ## Sections
///
/// - `global { … }`: global-only `:set` keys.
///   `"key" => field: Type = default, scope: [...], parser: kind [, resync: true];`
///   Declare `resync: true` when the write has a derived-state effect.
/// - `buffer { … }`: per-buffer-overridable `:set` keys; same format, no `resync:`.
/// - `extra_global { … }`: `EditorSettings`-only fields, no `:set` key.
///   `field: Type = default;`
/// - `extra_buffer { … }`: fields on both structs, no `:set` key.
///   `field: Type = global_default;` (buffer default is `None`)
/// - `subfield { … }`: a `:set` key stored in a nested field of an
///   `extra_global` value, with a differently named buffer override.
///   `"key" => global_field.sub_field / override_field : Type, scope: [...], parser: kind;`
///   Used by the three `whitespace-*` keys.
/// - `manual_keys { … }`: `:set` keys with a hand-written write arm below the
///   invocation; this section is still the sole source of their
///   [`setting_scopes`]/[`all_setting_keys`] entries. `"key" => [scope, ...];`
///   Used by `"statusline"`.
///
/// ## Parser kinds
///
/// | Token | Function |
/// |-------|----------|
/// | `bool` | `parse_bool(value)` |
/// | `usize` | `parse_usize(value)` |
/// | `usize_nonzero` | `parse_usize_nonzero(value)` |
/// | `tab_width` | `parse_tab_width(value)` |
/// | `from_str` | `SettingText::parse` (type inferred from field) |
/// | `string` | `value.to_owned()` |
/// | `show_newline` | `parse_show_newline(value)` (`none`/`all` wire format) |
/// | `word_chars` | `parse_word_chars(value)` (validated, unlike `string`) |
/// | `shell_command` | `ShellCommand`'s `SettingText::parse` for text, [`ShellCommand::from_words`] for a script's list; reads back as a list of strings |
///
/// Every kind but `shell_command` takes one value and refuses a list.
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

        /// Global editor settings: the authoritative defaults for all
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
            /// Not `pub`, unlike every other field here: `write_global`'s
            /// hand-written `"statusline"` arm is its only legal writer, so
            /// keeping the field itself private (rather than merely
            /// `pub(crate)`) makes a raw assignment from anywhere else in
            /// this crate (a raw assignment from anywhere else is a compile
            /// error instead of a bug). Read through [`EditorSettings::statusline`].
            statusline: StatusLineConfig,
            origins: SettingOrigins,
        }

        impl Default for EditorSettings {
            fn default() -> Self {
                Self {
                    $( $gname: $gdefault, )*
                    $( $bname: $bdefault, )*
                    $( $egname: $egdefault, )*
                    $( $ebname: $ebdefault, )*
                    statusline: StatusLineConfig::default(),
                    origins: SettingOrigins::default(),
                }
            }
        }

        impl EditorSettings {
            /// Read the resolved statusline config. There is no write
            /// counterpart on this type; see the field's own doc.
            pub fn statusline(&self) -> &StatusLineConfig {
                &self.statusline
            }
        }

        /// Per-buffer setting overrides. All fields are `Option<T>`; `None`
        /// means "inherit from the global [`EditorSettings`]".
        ///
        /// Resolution is always lazy: call the accessor (e.g.
        /// `tab_width`) with a `&EditorSettings` reference.
        #[derive(Default)]
        pub struct BufferOverrides {
            $( pub $bname: Option<$btype>, )*
            $( pub $ebname: Option<$ebtype>, )*
            $( pub $sfield: Option<$stype>, )*
            origins: SettingOrigins,
        }

        impl BufferOverrides {
            $( buffer_accessor!($bname, $btype, $bparser); )*
        }

        // ── write_global / write_buffer ───────────────────────────────────────

        /// Write a global setting's raw value from `by`, with no
        /// derived-state resync.
        ///
        /// Returns `Err(message)` on unknown key or invalid value, checked
        /// before the rank, and `Ok(false)`, writing nothing, when a higher
        /// origin wrote the value ([`SettingOrigins`]).
        ///
        /// This is the raw field write only. Some settings have derived
        /// state that must be resynced after a successful write (declared
        /// via `resync: true` above). Production code must go through
        /// [`ops::apply_global`], which wraps this and runs those effects;
        /// calling this directly would silently skip them.
        ///
        /// `pub(in crate::editor::settings)`: [`ops::apply_global`] is the
        /// one production caller, with editor state to resync against, and
        /// `settings::tests` is the only other reach. `testing::MockHost`
        /// needs the raw write too (it has no `EditorState`/`EngineView` to
        /// resync effects against) but lives outside this module, so it goes
        /// through `ops::write_global_for_test`, a `#[cfg(test)]`-gated
        /// `pub(crate)` pass-through that does not exist in a production
        /// build, not a widening of this function's own visibility.
        pub(in crate::editor::settings) fn write_global(
            key: &str,
            value: &OptionInput,
            settings: &mut EditorSettings,
            by: ConfigOrigin,
        ) -> Result<bool, String> {
            match key {
                $( $gkey => {
                    let parsed = parse_setting!(value, key, $gparser)?;
                    if !settings.origins.claim($gkey, by) { return Ok(false); }
                    settings.$gname = parsed;
                } )*
                $( $bkey => {
                    let parsed = parse_setting!(value, key, $bparser)?;
                    if !settings.origins.claim($bkey, by) { return Ok(false); }
                    settings.$bname = parsed;
                } )*
                $( $skey => {
                    let parsed = parse_setting!(value, key, $sparser)?;
                    if !settings.origins.claim($skey, by) { return Ok(false); }
                    settings.$sglobal.$ssub = parsed;
                } )*
                // Statusline config, global-only: `:set`'s text or a script's sections.
                "statusline" => {
                    let parsed = for_key(key, parse_statusline(value))?;
                    if !settings.origins.claim("statusline", by) { return Ok(false); }
                    settings.statusline = parsed;
                }
                _ => return Err(format!("unknown setting '{key}'")),
            }
            Ok(true)
        }

        /// Write a buffer-scoped setting's raw override, with no derived-state
        /// resync (no buffer-scoped key has one today; see [`write_global`]'s
        /// doc for the mechanism global-only keys use).
        ///
        /// Returns `Err(message)` on unknown key, a global-only key, or an
        /// invalid value, and `Ok(false)`, writing nothing, when a higher
        /// origin wrote the override ([`SettingOrigins`]).
        ///
        /// `pub(in crate::editor::settings)`: [`ops::apply_buffer`] is the
        /// only caller. `testing::MockHost` models no buffers, so it has no
        /// per-buffer override to write and needs no forwarding shim here
        /// (contrast [`write_global`]'s `write_global_for_test`).
        pub(in crate::editor::settings) fn write_buffer(
            key: &str,
            value: &OptionInput,
            overrides: &mut BufferOverrides,
            by: ConfigOrigin,
        ) -> Result<bool, String> {
            match key {
                $( $bkey => {
                    let parsed = parse_setting!(value, key, $bparser)?;
                    if !overrides.origins.claim($bkey, by) { return Ok(false); }
                    overrides.$bname = Some(parsed);
                } )*
                $( $skey => {
                    let parsed = parse_setting!(value, key, $sparser)?;
                    if !overrides.origins.claim($skey, by) { return Ok(false); }
                    overrides.$sfield = Some(parsed);
                } )*
                "statusline" => {
                    return Err("'statusline' is a global-only setting; use :set global statusline=…".to_string());
                }
                $( $gkey => {
                    return Err(format!(
                        "'{key}' is a global-only setting; use :set global {key}=…"
                    ));
                } )*
                _ => return Err(format!("unknown setting '{key}'")),
            }
            Ok(true)
        }

        /// Decode `key` into its [`ResyncKey`] variant, or `None` if it
        /// doesn't declare `resync: true` above. The sole source both
        /// `editor::settings::ops::resync_derived_state` (which key to
        /// resync) and `reset_globals` (which keys need resyncing at all)
        /// go through. See [`ResyncKey`]'s own doc for the compile-time
        /// property this buys.
        pub(in crate::editor) fn resync_key(key: &str) -> Option<ResyncKey> {
            match key {
                $( $( $gkey if $gresync => Some(ResyncKey::$gname), )? )*
                _ => None,
            }
        }

        // ── setting_value (get-option / get-buffer-option) ─────────────────────

        /// The effective value of `key`: `overrides`' value if `Some` and
        /// the key is buffer-scoped, else the global default. Backs
        /// `(get-option key)` (`overrides` always `None`: global only) and
        /// `(get-buffer-option bid key)` (`overrides` from `bid`'s stored
        /// `BufferOverrides`). `None` for a key with no generic storage,
        /// such as a `BufferIntrinsicOption`: the `get-buffer-option`
        /// builtin routes those to their own readers before reaching here.
        pub fn setting_value(
            key: &str,
            settings: &EditorSettings,
            overrides: Option<&BufferOverrides>,
        ) -> Option<hume_scripting::host::OptionValue> {
            match key {
                $( $gkey => Some(option_value!(settings.$gname.clone(), $gparser)), )*
                // `option_value!` is called separately in each branch, not
                // hoisted after a shared `let value = …`: the `word_chars`
                // accessor returns `&str` while the global fallback is an
                // owned `String` clone, so the two branches don't unify to
                // one type the way every `Copy` buffer setting's do.
                $( $bkey => match overrides {
                    Some(o) => Some(option_value!(o.$bname(settings), $bparser)),
                    None => Some(option_value!(settings.$bname.clone(), $bparser)),
                }, )*
                $( $skey => {
                    let value = match overrides {
                        Some(o) => o.$sfield.unwrap_or(settings.$sglobal.$ssub),
                        None => settings.$sglobal.$ssub,
                    };
                    Some(option_value!(value, $sparser))
                } )*
                "statusline" => Some(hume_scripting::host::OptionValue::Sections(
                    statusline_sections(&settings.statusline),
                )),
                _ => None,
            }
        }

        /// The `Scope`s a setting accepts, as declared by its `scope: [...]`
        /// list in the `define_settings!` invocation below. Empty for any
        /// key not declared there, notably the `BufferIntrinsicOption`s,
        /// which have no generic storage: `typed_set` and the option
        /// builtins match them before consulting this table.
        pub(in crate::editor) fn setting_scopes(key: &str) -> &'static [Scope] {
            match key {
                $( $gkey => &[$($gscope),+], )*
                $( $bkey => &[$($bscope),+], )*
                $( $skey => &[$($sscope),+], )*
                $( $mkey => &[$($mscope),+], )*
                _ => &[],
            }
        }

        /// Every setting key with a `:set` wire format: the union of the
        /// `global`/`buffer`/`subfield` macro entries and the `manual_keys`
        /// entries (`statusline`). Notably **excludes** the
        /// `BufferIntrinsicOption`s, which have no macro entry and are
        /// surfaced only when the completer knows the scope is `"buffer"`
        /// (their sole valid scope).
        pub(in crate::editor) fn all_setting_keys() -> &'static [&'static str] {
            &[$($gkey,)* $($bkey,)* $($skey,)* $($mkey,)*]
        }

        /// `true` if `key`'s value is parsed with `parser: bool`, i.e. its
        /// only valid values are `"true"`/`"false"`. Derived from the same
        /// per-key `parser: kind;` declaration used to dispatch parsing in
        /// `write_global`/`write_buffer`, so a new bool setting is picked up
        /// automatically by anything that queries this (e.g.
        /// `completion::complete_set`'s value completion) instead of
        /// needing a hand-copied key list. `manual_keys` never declare a
        /// `parser:`, so this only checks global/buffer/subfield.
        pub(in crate::editor) fn is_bool_setting(key: &str) -> bool {
            match key {
                $( $gkey => stringify!($gparser) == "bool", )*
                $( $bkey => stringify!($bparser) == "bool", )*
                $( $skey => stringify!($sparser) == "bool", )*
                _ => false,
            }
        }
    };
}

// A key names its area first (`indent-…`, `insert-…`, `lsp-…`), so `:set`'s
// prefix completion lists an area's settings together.
define_settings! {
    global {
        "scroll-margin" => scroll_margin: usize = 3,
            scope: [Scope::Global],
            parser: usize;
        "scroll-jump-align" => scroll_jump_align: ObjectJumpAlign = ObjectJumpAlign::default(),
            scope: [Scope::Global],
            parser: from_str;
        "insert-cursor-shape" => insert_cursor_shape: CursorShape = CursorShape::default(),
            scope: [Scope::Global],
            parser: from_str;
        "mouse-scroll-lines" => mouse_scroll_lines: usize = 3,
            scope: [Scope::Global],
            parser: usize;
        "mouse" => mouse: bool = true,
            scope: [Scope::Global],
            parser: bool;
        "mouse-select" => mouse_select: bool = false,
            scope: [Scope::Global],
            parser: bool;
        // Resizes every open pane's live jump list cap. Like undo-levels
        // below, takes effect on the next push, not retroactively. See
        // `editor::settings::ops::resync_derived_state` and `JumpList::set_capacity`.
        "jump-list-capacity" => jump_list_capacity: usize = 100,
            scope: [Scope::Global],
            parser: usize_nonzero,
            resync: true;
        "jump-list-min-lines" => jump_list_min_lines: usize = 5,
            scope: [Scope::Global],
            parser: usize;
        // Resizes the command/search prompt-history ring cap. Like
        // undo-levels below, takes effect on the next push, not
        // retroactively. See `editor::settings::ops::resync_derived_state`
        // and `History::set_capacity`.
        "prompt-history-levels" => prompt_history_levels: usize = 100,
            scope: [Scope::Global],
            parser: usize_nonzero,
            resync: true;
        // 0 is a valid, meaningful value here (unlimited), unlike
        // prompt-history-levels above, hence plain `usize`, not `usize_nonzero`.
        // Resizes the undo-tree cap on every open buffer. Takes effect on
        // the next edit, not retroactively (Vim's `undolevels` semantics).
        // See `editor::settings::ops::resync_derived_state`.
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
        // How long a write waits for `on-before-buffer-save` handlers that
        // took `done` before it goes ahead without them.
        "hook-before-save-timeout-ms" => hook_before_save_timeout_ms: usize = 1_000,
            scope: [Scope::Global],
            parser: usize_nonzero;
        "command-completion" => command_completion: CommandCompletion = CommandCompletion::default(),
            scope: [Scope::Global],
            parser: from_str;
        "popup-border" => popup_border: bool = true,
            scope: [Scope::Global],
            parser: bool;
        // Read by the completion menu's documentation box and by the
        // selection-time `completionItem/resolve` that fills it.
        "completion-docs" => completion_docs: bool = true,
            scope: [Scope::Global],
            parser: bool;
        "pane-dividers" => pane_dividers: bool = true,
            scope: [Scope::Global],
            parser: bool;
        // Read fresh by the statusline provider each frame; no resync needed.
        "statusline-mode-colors" => statusline_mode_colors: bool = true,
            scope: [Scope::Global],
            parser: bool;
        // Resolved into the tab bar's rows by `EditorState::screen_layout`, and
        // into the `TablineViewState` snapshot the provider paints by
        // `sync_tabline_view` every frame.
        "tabline" => tabline: TablineVisibility = TablineVisibility::default(),
            scope: [Scope::Global],
            parser: from_str;
        // Loads and applies the named theme immediately, rolling back to the
        // theme on screen on failure; empty selects the compiled-in fallback.
        // See `editor::settings::ops::resync_derived_state`.
        "theme" => theme: String = DEFAULT_THEME.to_string(),
            scope: [Scope::Global],
            parser: string,
            resync: true;
        // The program `pipe` runs each command line with.
        "shell" => shell: ShellCommand = ShellCommand::default(),
            scope: [Scope::Global],
            parser: shell_command;
        "syntax-highlight-max-bytes" => syntax_highlight_max_bytes: usize = 1_048_576,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // Scroll bursts (page-down held, mouse wheel) must collapse to one
        // OnViewportChange fire, not one per frame.
        "hook-viewport-debounce-ms" => hook_viewport_debounce_ms: usize = 150,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // rust-analyzer's first requests during indexing are slow; 10s
        // gives real-world servers room before the request is dropped as
        // TimedOut.
        "lsp-request-timeout-ms" => lsp_request_timeout_ms: usize = 10_000,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // A typing burst collapses to one diagnostics pull per buffer.
        "lsp-diagnostics-pull-debounce-ms" => lsp_diagnostics_pull_debounce_ms: usize = 250,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // Hint = most lenient: every severity renders. Gates the diagnostic
        // underline/extra-highlight and gutter-sign render write sides.
        "lsp-diagnostics-min-severity" => lsp_diagnostics_min_severity: DiagSeverity = DiagSeverity::Hint,
            scope: [Scope::Global],
            parser: from_str;
        // Off hides the diagnostic underline and end-of-line summary on the
        // primary cursor's line while typing in Insert mode. The summary is
        // hidden by `core:lsp` passing the negation to `set-eol-text!`.
        "lsp-diagnostics-on-insert-line" => lsp_diagnostics_on_insert_line: bool = false,
            scope: [Scope::Global],
            parser: bool;
    }
    buffer {
        // A buffer-overridable setting like any other (e.g. from an
        // `on-language-set` hook, for a per-filetype default), plus a third,
        // narrower rung: `scope` below additionally allows `Scope::Pane`:
        // `:set pane wrap-mode=…` (see `typed_file::typed_set`) writes
        // straight to the live `Pane`'s override, a separate path from
        // `write_global`/`write_buffer`, since wrap is also a view property
        // (two panes on the same buffer may wrap differently once one is
        // pinned; see `commands::effective_wrap_mode`, the pane → buffer →
        // global resolver every render/motion path reads through).
        "wrap-mode" => wrap_mode: WrapMode = hume_engine::pane::DEFAULT_WRAP_STYLE,
            scope: [Scope::Global, Scope::Buffer, Scope::Pane],
            parser: from_str;
        "indent-width" => indent_width: u8 = 4,
            scope: [Scope::Global, Scope::Buffer],
            parser: tab_width;
        "indent-guides" => indent_guides: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        "indent-style" => indent_style: TabStyle = TabStyle::default(),
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        "gutter-line-numbers" => gutter_line_numbers: LineNumberStyle = LineNumberStyle::default(),
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        "insert-auto-pairs" => insert_auto_pairs: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        // Enter, `o` and `O` indent a new line (see `break_line`), and a
        // typed closing token dedents its line (`electric_dedent`). Off, a
        // new line starts at the left margin and no line is dedented.
        "indent-smart" => indent_smart: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        // Leaving Insert mode selects whatever text the session just typed
        // (empty run: falls back to the entry command's own exit position).
        // See `begin_typed_run` and `tear_down_insert`'s pinned-anchor
        // finalization. Applies to every way of entering Insert mode:
        // `i`/`a`/`I`/`A`/`o`/`O`/`c`.
        "insert-select-on-exit" => insert_select_on_exit: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        // `toggle-comment` on a selection spanning several lines takes a
        // line token per line instead of the language's block pair.
        "comment-prefer-line" => comment_prefer_line: bool = false,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        // Word motions (`w`/`W`/`b`/`B`), `mm`/`MM` and `maw`/`maW` cover
        // the destination word's whitespace bookend on the preferred side.
        // See `word_select_cmd`'s `ctx.whitespace` read and `run_body`'s
        // `SelectionBody::Word` arm, which resolves it.
        "word-selects-whitespace" => word_selects_whitespace: WordSelectsWhitespace = WordSelectsWhitespace::default(),
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        // Extra characters this buffer counts as part of a word, on top of
        // the built-in alphanumeric-plus-`_` rule (Vim's `iskeyword`, minus
        // the range syntax), e.g. `-` makes `foo-bar` one word in CSS.
        // Affects `w`/`b`, `mm`, `miw`/`maw`, `select-word-nearest-on-line`,
        // Ctrl-w, `*`, `(symbol-under-cursor)`, symmetric auto-pair
        // suppression, and the LSP completion fallback replace span (no
        // server `textEdit`); the classifier itself is
        // `hume_editing::word::WordChars`. Does NOT affect `W`/`B`/`MM`:
        // they already merge punctuation into `Word` before comparing, so
        // widening `Word` further is a no-op for them. No global default per
        // language ships; set this per-language from an `on-language-set`
        // hook (see `configuration.md`). Whitespace (any `char::is_whitespace`
        // char, not just the five `classify_char` calls blank) is rejected
        // at write time, since promoting one to `Word` would leave a word run
        // with no terminator (see `WordChars::validate`).
        "word-chars" => word_chars: String = String::new(),
            scope: [Scope::Global, Scope::Buffer],
            parser: word_chars;
        "gutter-signs" => gutter_signs: SignColumnConfig = SignColumnConfig::default(),
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        // Read fresh by `check_buffer_disk_state` at each trigger; no
        // resync needed. `true`: an external change to the focused buffer
        // opens a reload confirm. `false`: detection still runs and warns,
        // but reload stays manual via `:e!`/`:checktime`. Independent of
        // `:w`'s write guard, which reads the file itself at write time
        // regardless of this setting.
        "file-reload-prompt" => file_reload_prompt: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
    }
    extra_global {
        // `statusline` is declared directly on the struct template above,
        // not here: `extra_global` fields are all `pub`, and `statusline`
        // must not be.
        //
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
        // Parsed via parse_statusline, not FromStr: global-only.
        "statusline" => [Scope::Global];
    }
}

/// Parse a `statusline` value: `:set`'s `"left|center|right"` text, or the
/// sections a script passes.
///
/// The text requires three `|`-separated sections, no more, each a
/// comma-separated list of `StatusElement` names. Either way an empty
/// section is allowed, an empty name inside one (`Mode,,FileName`) is not.
fn parse_statusline(value: &OptionInput) -> Result<StatusLineConfig, String> {
    let names = |section: &str| -> Vec<String> {
        if section.is_empty() {
            Vec::new()
        } else {
            section.split(',').map(str::to_string).collect()
        }
    };
    let parsed;
    let sections = match value {
        OptionInput::Sections(sections) => sections,
        OptionInput::Text(text) => {
            let parts: Vec<&str> = text.splitn(4, '|').collect();
            let [left, center, right] = parts.as_slice() else {
                return Err(format!(
                    "expected three sections separated by '|' \
                     (e.g. 'Mode,FileName||Position'), got '{text}'"
                ));
            };
            parsed = StatuslineSections {
                left: names(left),
                center: names(center),
                right: names(right),
            };
            &parsed
        }
        OptionInput::Words(_) => {
            return Err("takes three sections, not a list".to_string());
        }
    };
    let elements = |names: &[String]| -> Result<Vec<StatusElement>, String> {
        names
            .iter()
            .map(|name| {
                if name.is_empty() {
                    return Err(format!(
                        "section '{}' has an empty element name",
                        names.join(",")
                    ));
                }
                name.parse::<StatusElement>()
            })
            .collect()
    };
    Ok(StatusLineConfig {
        left: elements(&sections.left)?,
        center: elements(&sections.center)?,
        right: elements(&sections.right)?,
    })
}

/// The element names of each of `cfg`'s sections, as `get-option` reads them:
/// [`parse_statusline`]'s inverse.
fn statusline_sections(cfg: &StatusLineConfig) -> StatuslineSections {
    let names = |elems: &[StatusElement]| elems.iter().map(ToString::to_string).collect();
    StatuslineSections {
        left: names(&cfg.left),
        center: names(&cfg.center),
        right: names(&cfg.right),
    }
}

/// The wire-format strings [`parse_show_newline`] accepts: the single
/// source `:set buffer whitespace-newline=<Tab>` completion mirrors (see
/// `editor::completion::set::static_value_candidates`), so the two can never
/// drift out of sync. The sibling `space`/`tab` settings get the same
/// guarantee from `WhitespaceRender`'s `settings_enum!`.
pub(in crate::editor) const SHOW_NEWLINE_VALUES: &[&str] = &["none", "all"];

/// Parse the `whitespace-newline` wire format. Unlike `space`/`tab`, a
/// newline is inherently always at end-of-line, so there's no meaningful
/// "trailing" distinction, only `none`/`all`.
fn parse_show_newline(s: &str) -> Result<bool, String> {
    parse_named(s, SHOW_NEWLINE_VALUES, [false, true])
}

/// Render a `whitespace-newline` value back to the wire format
/// [`parse_show_newline`] accepts: the inverse.
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
    pub(in crate::editor) fn whitespace(&self, global: &EditorSettings) -> WhitespaceConfig {
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
    /// and only `insert-auto-pairs` is an actual per-buffer setting.
    pub(in crate::editor) fn auto_pairs_ref(
        &self,
        global: &EditorSettings,
    ) -> (bool, &'static [Pair]) {
        (
            self.insert_auto_pairs(global),
            hume_ops::auto_pairs::DEFAULT_PAIRS,
        )
    }
}

// ── Value parsers ─────────────────────────────────────────────────────────────

fn parse_usize(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("expected a non-negative integer, got '{value}'"))
}

fn parse_usize_nonzero(value: &str) -> Result<usize, String> {
    let n = parse_usize(value)?;
    if n == 0 {
        return Err("must be at least 1".into());
    }
    Ok(n)
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(format!("expected true/false, got '{value}'")),
    }
}

fn parse_tab_width(value: &str) -> Result<u8, String> {
    let n: u8 = value
        .parse()
        .map_err(|_| format!("expected 1–255, got '{value}'"))?;
    if n == 0 {
        return Err("must be at least 1".into());
    }
    Ok(n)
}

/// `parser: string` (`Ok(value.to_owned())`, unconditionally) is not usable
/// here: a `word-chars` value must reject whitespace/newline (see
/// `WordChars::validate`), so it gets its own parser kind instead of the
/// generic unvalidated string one.
fn parse_word_chars(value: &str) -> Result<String, String> {
    hume_editing::word::WordChars::validate(value)?;
    Ok(value.to_owned())
}

/// Applying a setting change: the single production path
/// ([`ops::apply_global`]/[`ops::apply_buffer`]). A child of this module,
/// not a sibling, so [`write_global`]/[`write_buffer`] above narrow to
/// `pub(in crate::editor::settings)`: the chokepoint this crate's write path
/// funnels through is reachable from exactly `settings::ops` and
/// `settings::tests`, not from every one of the ~110 other files under
/// `crate::editor`. The module path itself is `pub(crate)`, wider than that,
/// only so `testing::mock_host` (outside `crate::editor` entirely) can
/// name `ops::write_global_for_test`; every other item in `ops` keeps its
/// own narrower per-item visibility regardless of the path being nameable.
pub(crate) mod ops;

mod values;
mod word_list;

pub use values::{
    CommandCompletion, CursorShape, ObjectJumpAlign, SignColumnConfig, SignColumnMode,
    TablineVisibility, WordSelectsWhitespace,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod manual_options_drift;
