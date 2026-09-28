//! Editor settings: the single source of truth for configurable behaviour.
//!
//! [`EditorSettings`] holds a concrete value for every setting.
//! [`BufferOverrides`] lives on each `Buffer` with an `Option<T>` per
//! overridable setting (`None` inherits the global), resolved at call time by
//! its accessors; no merged copy is kept.
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
//! `language` has no macro entry: it has no global default, and its write
//! needs `Editor`-level access (`OnLanguageSet` hook, registry lookup), so it
//! is a special case in `typed_set`.

use std::fmt;
use std::str::FromStr;

use hume_editing::tab_style::TabStyle;
use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};

use crate::statusline::{StatusElement, StatusLineConfig};
use hume_ops::auto_pairs::Pair;

// ── settings_enum! ────────────────────────────────────────────────────────────

/// `"a"`, `"a or b"`, `"a, b, or c"`: how [`settings_enum`]'s parse error
/// lists the values it would have accepted.
fn or_list(values: &[&str]) -> String {
    match values {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [a, b] => format!("{a} or {b}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

/// Generate a `:set` enum's wire-format plumbing from one list of
/// `Variant => "name"` pairs: the `VALUES` slice `:set <key>=<Tab>` completes
/// from, a case-insensitive `FromStr` whose error names every accepted value,
/// and the `Display` that writes the same names back.
///
/// Hand-writing these meant four copies of one variant list per enum (the
/// const, the parse arms, the error message's prose, the display arms) and
/// four places for a new variant to be half-added. A variant missing from
/// `VALUES` alone still parses and prints, so it fails silently: it just stops
/// being completable. Only the enum declaration stays hand-written, so each
/// variant keeps its own doc comment.
macro_rules! settings_enum {
    ($ty:ty, $key:literal, [$($variant:ident => $name:literal),+ $(,)?]) => {
        impl $ty {
            /// The wire-format strings [`FromStr`] accepts: the single source
            /// `:set <key>=<Tab>` completion mirrors, so the two can never
            /// drift out of sync.
            pub const VALUES: &'static [&'static str] = &[$($name),+];
        }

        impl FromStr for $ty {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s.to_ascii_lowercase().as_str() {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(format!(
                        concat!("invalid ", $key, " '{}': expected {}"),
                        s,
                        or_list(<$ty>::VALUES),
                    )),
                }
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self {
                    $(Self::$variant => $name,)+
                })
            }
        }
    };
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

/// Sign column configuration: visibility mode and, optionally, a pinned
/// number of sign slots.
///
/// Wire format: `"always"`, `"always:N"`, `"auto"`, `"auto:N"` where N is the
/// number of sign slots (1–127). Bare `"always"`/`"auto"` (`pinned_slots:
/// None`) auto-sizes the column to the buffer's registered sign sources,
/// one slot per source, regardless of whether it has placed a sign anywhere
/// in the buffer (see `DecorationStores::sign_source_count`, the sole place
/// `slots_for`'s `None` branch reads); `":N"` pins the count instead,
/// hiding any source ranked at or past `N`. Default is bare `"always"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignColumnConfig {
    pub mode: SignColumnMode,
    /// Explicit slot count from a `:N` suffix, or `None` to auto-size.
    pub pinned_slots: Option<u8>,
}

impl Default for SignColumnConfig {
    fn default() -> Self {
        Self {
            mode: SignColumnMode::Always,
            pinned_slots: None,
        }
    }
}

impl SignColumnConfig {
    /// Upper bound on a pinned `:N` slot count, and on how many slots bare
    /// `signcolumn=always`/`auto` auto-sizes to. The `:N` bound is the type
    /// domain `FromStr` parses into (`u8`, and `127` keeps a pinned column
    /// from outrunning what a terminal row can usefully show); auto-size
    /// reuses the same constant rather than defining its own smaller cap,
    /// since a buffer can't register more distinct sources than a user could
    /// otherwise pin explicitly.
    pub const MAX_SLOTS: u8 = 127;

    /// Resolves the configured slot count against `source_count`, the
    /// number of registered sign sources. An explicit `:N` pins the count
    /// regardless of `source_count`; auto-size clamps to `[1, MAX_SLOTS]`,
    /// never below 1, so the column stays visible under `always` even with
    /// zero registered sources. Returns a bare slot count, not a gutter
    /// width: `Editor::update_sign_providers` (its only caller) needs the
    /// bare count to bound the per-line `Vec<Sign>` it builds, and leaves the
    /// `+1` padding-column conversion to `SignColumn::width_for_slots`,
    /// applied later at the point the resolved width is actually synced to
    /// the gutter.
    pub fn slots_for(self, source_count: usize) -> u8 {
        self.pinned_slots
            .unwrap_or_else(|| source_count.clamp(1, Self::MAX_SLOTS as usize) as u8)
    }

    /// Completion hints only, not an exhaustive enum like `TabStyle::VALUES`
    /// because `:N` accepts 1–127, which can't be listed in full. `:1`/`:2` are
    /// illustrative of the pinned-vs-auto-size distinction, not a reflection
    /// of `MAX_SLOTS`; raising that cap doesn't require extending this list.
    pub const VALUES: &'static [&'static str] =
        &["always", "auto", "always:1", "auto:1", "always:2", "auto:2"];
}

impl fmt::Display for SignColumnConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.pinned_slots {
            Some(n) => write!(f, "{}:{}", self.mode, n),
            None => write!(f, "{}", self.mode),
        }
    }
}

impl FromStr for SignColumnConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (mode_str, slots_str) = match s.split_once(':') {
            Some((m, c)) => (m, Some(c)),
            None => (s, None),
        };
        let mode: SignColumnMode = mode_str.parse()?;
        let pinned_slots = match slots_str {
            Some(c) => {
                let n: u8 = c
                    .parse()
                    .map_err(|_| format!("invalid signcolumn slots: expected 1–127, got '{c}'"))?;
                if n == 0 || n > Self::MAX_SLOTS {
                    return Err(format!(
                        "invalid signcolumn slots: expected 1–127, got '{n}'"
                    ));
                }
                Some(n)
            }
            None => None,
        };
        Ok(Self { mode, pinned_slots })
    }
}

/// Where a forward object jump (`}`, `goto-next-<kind>`) leaves the viewport.
///
/// Exists because those motions land the selection head at the *start* of
/// the object just found (`hume_ops::motion::object::apply_object_motion`'s
/// `Selection::new(end, start)`, deliberately, so a following `w` walks into
/// the object's body), and the body then extends *below* that head. A
/// forward jump scrolls downward, so the default per-frame scroll parks the
/// head at `scroll-margin` rows from the *bottom*, hiding the very body the
/// head-first convention was chosen to show. The backward motions (`{`)
/// don't have this problem: their head lands at the object's start too, but
/// an upward scroll parks it at `scroll-margin` rows from the *top*, so the body
/// below it is already on screen. Hence only the forward motions read this
/// setting; see `CmdMeta::aligns_view` in `editor::registry::command`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectJumpAlign {
    /// Pin the head at the viewport's top row, subject to `scroll-margin` on
    /// the next frame, exactly like `z k`.
    Top,
    /// Center the head in the viewport, like `z z`.
    #[default]
    Center,
    /// No extra alignment: the pre-existing per-frame `scroll-margin` scroll is
    /// all that runs.
    Off,
}

settings_enum!(ObjectJumpAlign, "object-jump-align", [
    Top => "top",
    Center => "center",
    Off => "off",
]);

/// The real terminal cursor's shape in Insert mode: Helix's
/// `editor.cursor-shape.insert`, minus the `hidden` variant Helix offers
/// mainly for IME positioning. Applies to every selection head, not just the
/// primary (HUME's own departure from Helix). See the Tier 1/0 comment in
/// `hume_engine::style::style_display_line` for why and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// Every selection head is painted from its own themed scope (the
    /// primary from `ui.cursor.primary.insert`, a secondary from
    /// `ui.cursor.insert`, both falling back through the usual ladder); the
    /// real terminal cursor is hidden.
    Block,
    /// Every selection head is left unpainted. The real terminal cursor
    /// shows as a thin bar, marking the primary; secondary heads are visible
    /// only where they fall inside a highlighted selection. This is HUME's
    /// long-standing default.
    #[default]
    Bar,
    /// Same as `Bar`, but the real terminal cursor shows as an underline.
    Underline,
}

settings_enum!(CursorShape, "cursor-shape-insert", [
    Block => "block",
    Bar => "bar",
    Underline => "underline",
]);

/// When to show the tab bar (top row of the terminal area).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TablineVisibility {
    /// Always shown, even with a single tab open.
    Always,
    /// Never shown, regardless of how many tabs are open.
    Never,
    /// Shown only once more than one tab is open. HUME's default.
    #[default]
    Dynamic,
}

settings_enum!(TablineVisibility, "tabline", [
    Always => "always",
    Never => "never",
    Dynamic => "dynamic",
]);

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
    history_capacity,
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
    ($value:expr, $key:expr, enum_str) => {
        $value.parse()
    };
    ($value:expr, $key:expr, string) => {
        Ok::<String, String>(($value).to_owned())
    };
    ($value:expr, $key:expr, show_newline) => {
        parse_show_newline($value)
    };
    ($value:expr, $key:expr, word_chars) => {
        parse_word_chars($value)
    };
}

/// Dispatch from a parser-kind token to the `get-option`-facing
/// [`hume_scripting::host::OptionValue`] shape. Mirrors [`parse_setting!`]'s
/// kind table so every setting stays readable the moment it's declared:
/// `bool` fields round-trip as `Bool`, integer-ish fields (`usize`,
/// `usize_nonzero`, `tab_width`) as `Int`, and closed sets of names
/// (`enum_str`) as `Symbol`. The remaining `from_str` and `string` fields go
/// via `Display`/`ToString` as `Str`: `wrap-mode` and `signcolumn` take
/// `name:N` forms, so they are not closed sets. `from_str` and `enum_str`
/// types must implement `Display` that round-trips through their own
/// `FromStr` (see `TabStyle`, `DiagSeverity`, `LineNumberStyle`, `WrapMode`).
/// `show_newline` stores a plain `bool` but its wire format is `none`/`all`,
/// so it round-trips as `Symbol` via [`format_show_newline`], the inverse of
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
    ($value:expr, enum_str) => {
        hume_scripting::host::OptionValue::Symbol($value.to_string())
    };
    ($value:expr, string) => {
        hume_scripting::host::OptionValue::Str($value)
    };
    ($value:expr, show_newline) => {
        hume_scripting::host::OptionValue::Symbol(format_show_newline($value).to_string())
    };
    ($value:expr, word_chars) => {
        hume_scripting::host::OptionValue::Str($value.to_string())
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
/// | `bool` | `parse_bool(value, key)` |
/// | `usize` | `parse_usize(value, key)` |
/// | `usize_nonzero` | `parse_usize_nonzero(value, key)` |
/// | `tab_width` | `parse_tab_width(value)` |
/// | `from_str` | `value.parse()` (type inferred from field) |
/// | `enum_str` | `value.parse()`, for a closed set of names; reads back as a symbol |
/// | `string` | `value.to_owned()` |
/// | `show_newline` | `parse_show_newline(value)` (`none`/`all` wire format, reads back as a symbol) |
/// | `word_chars` | `parse_word_chars(value)` (validated, unlike `string`) |
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
        }

        impl Default for EditorSettings {
            fn default() -> Self {
                Self {
                    $( $gname: $gdefault, )*
                    $( $bname: $bdefault, )*
                    $( $egname: $egdefault, )*
                    $( $ebname: $ebdefault, )*
                    statusline: StatusLineConfig::default(),
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
        }

        impl BufferOverrides {
            $( buffer_accessor!($bname, $btype, $bparser); )*
        }

        // ── write_global / write_buffer ───────────────────────────────────────

        /// Write a global setting's raw value, with no derived-state resync.
        ///
        /// Returns `Err(message)` on unknown key or invalid value.
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
            value: &str,
            settings: &mut EditorSettings,
        ) -> Result<(), String> {
            match key {
                $( $gkey => { settings.$gname = parse_setting!(value, key, $gparser)?; } )*
                $( $bkey => { settings.$bname = parse_setting!(value, key, $bparser)?; } )*
                $( $skey => { settings.$sglobal.$ssub = parse_setting!(value, key, $sparser)?; } )*
                // Statusline config, global-only; three sections separated by `|`,
                // each a comma-separated list of StatusElement names (may be empty).
                "statusline" => { settings.statusline = parse_statusline(value)?; }
                _ => return Err(format!("unknown setting '{key}'")),
            }
            Ok(())
        }

        /// Write a buffer-scoped setting's raw override, with no derived-state
        /// resync (no buffer-scoped key has one today; see [`write_global`]'s
        /// doc for the mechanism global-only keys use).
        ///
        /// Returns `Err(message)` on unknown key, a global-only key, or an
        /// invalid value.
        ///
        /// `pub(in crate::editor::settings)`: [`ops::apply_buffer`] is the
        /// only caller. `testing::MockHost` models no buffers, so it has no
        /// per-buffer override to write and needs no forwarding shim here
        /// (contrast [`write_global`]'s `write_global_for_test`).
        pub(in crate::editor::settings) fn write_buffer(key: &str, value: &str, overrides: &mut BufferOverrides) -> Result<(), String> {
            match key {
                $( $bkey => { overrides.$bname = Some(parse_setting!(value, key, $bparser)?); } )*
                $( $skey => { overrides.$sfield = Some(parse_setting!(value, key, $sparser)?); } )*
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
            Ok(())
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
        /// `BufferOverrides`). `None` for a key with no generic storage.
        /// This covers only `"language"` today, which lives on the buffer's
        /// language identity; `get-buffer-option` reads it itself.
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
                "statusline" => Some(hume_scripting::host::OptionValue::Str(
                    format_statusline(&settings.statusline),
                )),
                _ => None,
            }
        }

        /// The `Scope`s a setting accepts, as declared by its `scope: [...]`
        /// list in the `define_settings!` invocation below. Empty for any
        /// key not declared there, notably `"language"`, which has no
        /// generic storage and is handled entirely by `typed_set`'s own
        /// special case, never through this table.
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
        /// entries (`statusline`). Notably **excludes** `"language"`, which
        /// has no macro entry and is surfaced only when the completer knows
        /// the scope is `"buffer"` (its sole valid scope).
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

define_settings! {
    global {
        "scroll-margin" => scroll_margin: usize = 3,
            scope: [Scope::Global],
            parser: usize;
        "object-jump-align" => object_jump_align: ObjectJumpAlign = ObjectJumpAlign::Center,
            scope: [Scope::Global],
            parser: enum_str;
        "cursor-shape-insert" => cursor_shape_insert: CursorShape = CursorShape::Bar,
            scope: [Scope::Global],
            parser: enum_str;
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
        "jump-line-threshold" => jump_line_threshold: usize = 5,
            scope: [Scope::Global],
            parser: usize;
        // Resizes the command/search prompt-history ring cap. Like
        // undo-levels below, takes effect on the next push, not
        // retroactively. See `editor::settings::ops::resync_derived_state`
        // and `History::set_capacity`.
        "history-capacity" => history_capacity: usize = 100,
            scope: [Scope::Global],
            parser: usize_nonzero,
            resync: true;
        // 0 is a valid, meaningful value here (unlimited), unlike
        // history-capacity above, hence plain `usize`, not `usize_nonzero`.
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
        "popup-border" => popup_border: bool = true,
            scope: [Scope::Global],
            parser: bool;
        "pane-dividers" => pane_dividers: bool = true,
            scope: [Scope::Global],
            parser: bool;
        // Read fresh by the statusline provider each frame; no resync needed.
        "statusline.mode-colors" => statusline_mode_colors: bool = true,
            scope: [Scope::Global],
            parser: bool;
        // Resolved into `TablineViewState.visible` by `sync_tabline_view`
        // every frame. The provider itself reads that snapshot, not this
        // setting directly.
        "tabline" => tabline: TablineVisibility = TablineVisibility::default(),
            scope: [Scope::Global],
            parser: enum_str;
        // Loads and applies the named theme immediately, rolling back to the
        // previous value on failure. See
        // `editor::settings::ops::resync_derived_state`.
        "theme" => theme: String = String::new(),
            scope: [Scope::Global],
            parser: string,
            resync: true;
        "syntax-highlight-max-bytes" => syntax_highlight_max_bytes: usize = 1_048_576,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // rust-analyzer's first requests during indexing are slow; 10s
        // gives real-world servers room before the request is dropped as
        // TimedOut.
        "lsp.request-timeout-ms" => lsp_request_timeout_ms: usize = 10_000,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // One round trip per range, so a stray select-all-matches leaving
        // hundreds of cursors would burst hundreds of requests at the
        // server; past this many `:lsp-fmt` warns and formats nothing,
        // rather than silently narrowing to one selection. Doesn't bound a
        // server advertising `rangesSupport`: every range there rides one
        // `rangesFormatting` request, so there's nothing to burst.
        "lsp.format-max-ranges" => lsp_format_max_ranges: usize = 16,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // Scroll bursts (page-down held, mouse wheel) must collapse to one
        // OnViewportChange fire, not one per frame.
        "lsp.viewport-debounce-ms" => lsp_viewport_debounce_ms: usize = 150,
            scope: [Scope::Global],
            parser: usize_nonzero;
        // Hint = most lenient: every severity renders. Gates the diagnostic
        // underline/extra-highlight and gutter-sign render write sides.
        "lsp.diagnostics-severity-floor" => lsp_diagnostics_severity_floor: crate::editor::lsp::diagnostics::DiagSeverity = crate::editor::lsp::diagnostics::DiagSeverity::Hint,
            scope: [Scope::Global],
            parser: enum_str;
        // Gates the inlay-hint render write side. Off means the
        // `inlay_hints` store is untouched but nothing renders.
        "lsp.inlay-hints" => lsp_inlay_hints: bool = false,
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
        "tab-width" => tab_width: u8 = 4,
            scope: [Scope::Global, Scope::Buffer],
            parser: tab_width;
        "indent-guides" => show_indent_guides: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        "tab-style" => tab_style: TabStyle = TabStyle::Hard,
            scope: [Scope::Global, Scope::Buffer],
            parser: enum_str;
        "line-number-style" => line_number_style: LineNumberStyle = LineNumberStyle::Hybrid,
            scope: [Scope::Global, Scope::Buffer],
            parser: enum_str;
        "auto-pairs" => auto_pairs: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        // Leaving Insert mode selects whatever text the session just typed
        // (empty run: falls back to the entry command's own exit position).
        // See `begin_typed_run` and `end_insert_session`'s pinned-anchor
        // finalization. Applies to every way of entering Insert mode:
        // `i`/`a`/`I`/`A`/`o`/`O`/`c`.
        "select-inserted-text" => select_inserted_text: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
        // Word motions (`w`/`W`/`b`/`B`) and `mm`/`MM` cover the destination
        // word's whitespace bookend (leading, or trailing for the first
        // word of a line). See `word_select_cmd`'s `ctx.around` read and
        // `run_body`'s `SelectionBody::Word` arm, which resolves it.
        "word-selects-whitespace" => word_selects_whitespace: bool = true,
            scope: [Scope::Global, Scope::Buffer],
            parser: bool;
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
        "signcolumn" => signcolumn: SignColumnConfig = SignColumnConfig::default(),
            scope: [Scope::Global, Scope::Buffer],
            parser: from_str;
        // Read fresh by `check_buffer_disk_state` at each trigger; no
        // resync needed. `true`: an external change to the focused buffer
        // opens a reload confirm. `false`: detection still runs and warns,
        // but reload stays manual via `:e!`/`:checktime`. Independent of
        // `:w`'s write guard, which stats the file itself at write time
        // regardless of this setting. See `stale_write_block`.
        "auto-read" => auto_read: bool = true,
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
            parser: enum_str;
        "whitespace-tab" => whitespace.tab / whitespace_tab : WhitespaceRender,
            scope: [Scope::Global, Scope::Buffer],
            parser: enum_str;
        "whitespace-newline" => whitespace.newline / whitespace_newline : bool,
            scope: [Scope::Global, Scope::Buffer],
            parser: show_newline;
    }
    manual_keys {
        // Parsed via parse_statusline, not FromStr: global-only.
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
/// [`parse_statusline`] accepts: the inverse.
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

/// The wire-format strings [`parse_show_newline`] accepts: the single
/// source `:set buffer whitespace-newline=<Tab>` completion mirrors (see
/// `editor::completion::set::static_value_candidates`), so the two can never
/// drift out of sync. Mirrors the `WhitespaceRender::VALUES` pattern
/// (`hume-engine/src/pane.rs`) used by the sibling `space`/`tab` settings.
pub(in crate::editor) const SHOW_NEWLINE_VALUES: &[&str] = &["none", "all"];

/// Parse the `whitespace-newline` wire format. Unlike `space`/`tab`, a
/// newline is inherently always at end-of-line, so there's no meaningful
/// "trailing" distinction, only `none`/`all`.
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
    /// and only `auto-pairs` is an actual per-buffer setting.
    pub(in crate::editor) fn auto_pairs_ref(
        &self,
        global: &EditorSettings,
    ) -> (bool, &'static [Pair]) {
        (self.auto_pairs(global), hume_ops::auto_pairs::DEFAULT_PAIRS)
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod manual_options_drift;
