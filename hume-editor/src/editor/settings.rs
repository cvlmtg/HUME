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
//! `write_global`/`write_buffer`/`table_setting` dispatch and the readers.
//! A key's section gives its scopes, which [`SettingKey`] reads for every
//! caller that names a setting. A buffer key that a pane can also override
//! declares `pane: Variant`, naming its [`PaneSetting`]: neither generated
//! writer has pane storage, so `typed_set` and `typed_get` match that
//! variant. A global key whose write has a derived-state effect declares
//! `resync: true` (wired in `editor::settings::ops::apply_global`).
//!
//! The `BufferIntrinsicOption`s have no macro entry: they have no global
//! default and live on the buffer. `language` needs `Editor`-level access
//! (`OnLanguageSet` hook, registry lookup), so `ops::apply_buffer_target`
//! parses its value and returns the change, which `typed_set` makes and the
//! `set-buffer-option!` builtin queues. `ops::apply_buffer_target` writes
//! `line-ending`, and `ops::read_buffer_target` reads either back.

use std::fmt;
use std::hash::Hash;

use hume_core::{BufferIntrinsicOption, Claim, ConfigOrigin, DiagSeverity, LaterWins, OptionInput};
use hume_editing::tab_style::TabStyle;
use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};
use hume_platform::path::AbsolutePath;
use hume_platform::process::batch::ShellCommand;
use rustc_hash::FxHashMap;

use crate::statusline::StatusLineConfig;
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
/// Every value type behind a [`kind::Typed`] setting implements it,
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
#[derive(Clone, Debug)]
pub(in crate::editor::settings) struct SettingOrigins<K>(FxHashMap<K, Claim<LaterWins>>);

impl<K> Default for SettingOrigins<K> {
    fn default() -> Self {
        Self(FxHashMap::default())
    }
}

impl<K: Copy + Eq + Hash> SettingOrigins<K> {
    /// Whether `by` may write `key`; recorded as its writer when it may.
    fn claim(&mut self, key: K, by: ConfigOrigin) -> bool {
        self.0.entry(key).or_default().claim(by).is_applied()
    }

    /// `key`'s slot as it stands, for [`Self::restore`].
    pub(in crate::editor::settings) fn slot(&self, key: K) -> Claim<LaterWins> {
        self.0.get(&key).copied().unwrap_or_default()
    }

    /// Put back what [`Self::slot`] read for `key` before a write that is
    /// being undone.
    pub(in crate::editor::settings) fn restore(&mut self, key: K, slot: Claim<LaterWins>) {
        self.0.insert(key, slot);
    }
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
/// generic storage at all: a [`PaneSetting`] is written straight to the live
/// `Pane` by `typed_settings::typed_set`, a third, narrower rung *on top of*
/// `Buffer`/`Global` rather than a bypass of them: a pane with no pane-level
/// override still resolves through the buffer/global chain (see
/// `commands::effective_wrap_mode`).
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

    /// The scope `s` names, `None` for any other text.
    pub(in crate::editor) fn named(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|scope| scope.as_str() == s)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The active theme's setting.
pub(in crate::editor) const THEME: GlobalKey = GlobalKey::theme;

/// Name the `theme` setting starts as: the theme loaded from the search path
/// when config doesn't pick one.
pub(in crate::editor) const DEFAULT_THEME: &str = "sand";

/// One global setting key declared `resync: true` in `define_settings!`.
///
/// Only the generated [`GlobalKey::resync`] produces values, so drift is a
/// compile error both ways: a new `resync: true` key with no variant fails
/// there, and a variant with no arm fails `resync_derived_state`'s
/// exhaustive `match`. Variants reuse the snake_case field names because
/// `GlobalKey::resync` emits `$gname` verbatim (renaming would need `paste`).
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

/// A buffer setting a pane can also override, declared `pane: Variant` on its
/// `define_settings!` entry.
///
/// Only the generated [`BufferKey::pane`] produces values, and a `pane:`
/// clause naming no variant fails to compile there, so `typed_set` and
/// `typed_get` match every pane-scoped key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum PaneSetting {
    /// `wrap-mode`: the pane's own wrap override.
    WrapMode,
}

/// A `define_settings!` key, which always has a global value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(in crate::editor) enum TableKey {
    Global(GlobalKey),
    Buffer(BufferKey),
}

impl TableKey {
    /// Every settings-table key, the global-only ones first.
    pub(in crate::editor) fn all() -> impl Iterator<Item = Self> {
        GlobalKey::ALL
            .iter()
            .copied()
            .map(Self::Global)
            .chain(BufferKey::ALL.iter().copied().map(Self::Buffer))
    }

    /// The key `:set` and scripts name the setting by.
    pub(in crate::editor) const fn name(self) -> &'static str {
        match self {
            Self::Global(key) => key.name(),
            Self::Buffer(key) => key.name(),
        }
    }
}

/// What a setting names on one buffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum BufferTarget {
    /// An option the buffer stores itself.
    Intrinsic(BufferIntrinsicOption),
    /// A settings-table key's buffer override.
    Override(BufferKey),
}

/// Where a setting named in a scope lives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum SettingTarget {
    /// A settings-table key's global value.
    Global(TableKey),
    Buffer(BufferTarget),
    /// A setting's pane override.
    Pane(PaneSetting),
}

/// A setting by its key, and the scopes it takes: the one rule `:set`,
/// `:get`, their completion and the option builtins' host all read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum SettingKey {
    /// An option the buffer stores itself, valid only in buffer scope.
    Intrinsic(BufferIntrinsicOption),
    /// A settings-table key: a `global` or `private_global` key is valid
    /// only in global scope, a `buffer` or `subfield` key in global and
    /// buffer scope, and in pane scope when its `pane:` clause names a
    /// [`PaneSetting`].
    Table(TableKey),
}

impl SettingKey {
    /// The setting `key` names, `None` for an unknown key.
    pub(in crate::editor) fn named(key: &str) -> Option<Self> {
        BufferIntrinsicOption::from_name(key)
            .map(Self::Intrinsic)
            .or_else(|| table_setting(key).map(Self::Table))
    }

    /// Every setting, the buffer-intrinsic options first.
    pub(in crate::editor) fn all() -> impl Iterator<Item = Self> {
        BufferIntrinsicOption::ALL
            .into_iter()
            .map(Self::Intrinsic)
            .chain(TableKey::all().map(Self::Table))
    }

    /// The key `:set` and scripts name the setting by.
    pub(in crate::editor) const fn name(self) -> &'static str {
        match self {
            Self::Intrinsic(opt) => opt.name(),
            Self::Table(key) => key.name(),
        }
    }

    /// The setting's global value, `None` for one the buffer stores itself.
    pub(in crate::editor) fn global(self) -> Option<TableKey> {
        match self {
            Self::Table(key) => Some(key),
            Self::Intrinsic(_) => None,
        }
    }

    /// What the setting names on a buffer, `None` for a global-only one.
    pub(in crate::editor) fn buffer(self) -> Option<BufferTarget> {
        match self {
            Self::Intrinsic(opt) => Some(BufferTarget::Intrinsic(opt)),
            Self::Table(TableKey::Buffer(key)) => Some(BufferTarget::Override(key)),
            Self::Table(TableKey::Global(_)) => None,
        }
    }

    /// The setting's pane override, `None` for one a pane cannot override.
    pub(in crate::editor) fn pane(self) -> Option<PaneSetting> {
        match self {
            Self::Table(TableKey::Buffer(key)) => key.pane(),
            Self::Table(TableKey::Global(_)) | Self::Intrinsic(_) => None,
        }
    }

    /// Every scope the setting takes, in [`Scope::ALL`] order.
    pub(in crate::editor) fn scopes(self) -> impl Iterator<Item = Scope> {
        Scope::ALL
            .iter()
            .copied()
            .filter(move |&scope| self.target(scope).is_some())
    }

    /// Where the setting lives in `scope`, `None` for a scope it does not
    /// take.
    pub(in crate::editor) fn target(self, scope: Scope) -> Option<SettingTarget> {
        match scope {
            Scope::Global => self.global().map(SettingTarget::Global),
            Scope::Buffer => self.buffer().map(SettingTarget::Buffer),
            Scope::Pane => self.pane().map(SettingTarget::Pane),
        }
    }
}

/// The error for a key no setting has, whichever command or builtin named it.
pub(in crate::editor) fn unknown_setting(key: &str) -> String {
    format!("unknown setting '{key}'")
}

/// Whether a script reads or writes an option: which builtin a refusal of a
/// key in the wrong scope names instead.
#[derive(Clone, Copy)]
pub(in crate::editor) enum OptionAccess {
    Read,
    Write,
}

impl OptionAccess {
    /// The builtin that reaches an option's global value.
    fn global_builtin(self) -> &'static str {
        match self {
            Self::Read => "get-option",
            Self::Write => "set-option!",
        }
    }

    /// The builtin that reaches an option on one buffer.
    fn buffer_builtin(self) -> &'static str {
        match self {
            Self::Read => "get-buffer-option",
            Self::Write => "set-buffer-option!",
        }
    }
}

/// The setting a script's `key` names. `Err` for an unknown key.
fn script_setting(key: &str) -> Result<SettingKey, String> {
    SettingKey::named(key).ok_or_else(|| unknown_setting(key))
}

/// The global value a global builtin's `key` names. An option the buffer
/// stores itself has none, and its refusal names the buffer builtin.
pub(in crate::editor) fn global_key(key: &str, access: OptionAccess) -> Result<TableKey, String> {
    script_setting(key)?
        .global()
        .ok_or_else(|| format!("'{key}' is per-buffer: use {}", access.buffer_builtin()))
}

/// What a buffer builtin's `key` names on a buffer. A global-only option
/// has nothing there, and its refusal names the global builtin.
pub(in crate::editor) fn buffer_target(
    key: &str,
    access: OptionAccess,
) -> Result<BufferTarget, String> {
    script_setting(key)?
        .buffer()
        .ok_or_else(|| format!("'{key}' is global-only: use {}", access.global_builtin()))
}

/// Dispatch from a parser kind to a `BufferOverrides` accessor.
///
/// Every buffer setting but `word-chars` is `Copy`, so the default arm
/// resolves buffer-override-or-global by copying. `word_chars`
/// is the one `String`-typed buffer setting; borrowing instead of cloning is
/// what lets [`hume_editing::word::WordChars`] stay borrowed-and-`Copy` per
/// its own doc, rather than every caller paying a heap clone per keystroke.
macro_rules! buffer_accessor {
    ($bname:ident, $btype:ty, WordChars) => {
        /// Effective value: buffer override → global default.
        pub(in crate::editor) fn $bname<'a>(&'a self, global: &'a EditorSettings) -> &'a str {
            self.$bname.as_deref().unwrap_or(&global.$bname)
        }
    };
    ($bname:ident, $btype:ty, $bparser:ident) => {
        /// Effective value: buffer override → global default.
        pub(in crate::editor) fn $bname(&self, global: &EditorSettings) -> $btype {
            self.$bname.unwrap_or(global.$bname)
        }
    };
}

// ── Settings definition ───────────────────────────────────────────────────────

/// Generate [`EditorSettings`], [`BufferOverrides`], the key types
/// [`GlobalKey`] and [`BufferKey`], [`write_global`], [`write_buffer`] and
/// the readers [`read_global`] and [`read_buffer`] from a single source of
/// truth.
///
/// ## Sections
///
/// - `global { … }`: global-only `:set` keys.
///   `"key" => field: Type = default, parser: Kind [, resync: true];`
///   Declare `resync: true` when the write has a derived-state effect.
/// - `buffer { … }`: `:set` keys a buffer can override; same format, no
///   `resync:`. Declare `pane: Variant` (after `parser:`) for a key a pane can
///   also override: it adds `Scope::Pane` and names its [`PaneSetting`]. Only
///   this section takes it, so a pane-scoped key also takes buffer scope,
///   where a per-language `on-language-set` hook can reach it.
/// - `extra_global { … }`: `EditorSettings`-only fields, no `:set` key.
///   `field: Type = default;`
/// - `extra_buffer { … }`: fields on both structs, no `:set` key.
///   `field: Type = global_default;` (buffer default is `None`)
/// - `subfield { … }`: a `:set` key stored in a nested field of an
///   `extra_global` value, with a differently named buffer override.
///   `"key" => global_field.sub_field / override_field : Type, parser: Kind;`
///   Used by the three `whitespace-*` keys.
/// - `private_global { … }`: global-only `:set` keys whose field is private
///   and read through a getter of the same name, so a raw assignment outside
///   this module is a compile error and [`write_global`] is the only writer.
///   `"key" => field: Type, parser: Kind;` The default is `Type::default()`.
///   Used by `"statusline"`.
///
/// `parser:` names a type in [`kind`] that implements [`kind::Kind`] for
/// the field's type: how the value parses, completes and reads back.
macro_rules! define_settings {
    (
        new($start:ident)
        global {
            $( $gkey:literal => $gname:ident : $gtype:ty = $gdefault:expr, parser: $gparser:ident $(, resync: $gresync:literal)?; )*
        }
        buffer {
            $( $bkey:literal => $bname:ident : $btype:ty = $bdefault:expr, parser: $bparser:ident $(, pane: $bpane:ident)?; )*
        }
        extra_global {
            $( $egname:ident : $egtype:ty = $egdefault:expr; )*
        }
        extra_buffer {
            $( $ebname:ident : $ebtype:ty = $ebdefault:expr; )*
        }
        subfield {
            $( $skey:literal => $sglobal:ident . $ssub:ident / $sfield:ident : $stype:ty, parser: $sparser:ident; )*
        }
        private_global {
            $( $pkey:literal => $pname:ident : $ptype:ty, parser: $pparser:ident; )*
        }
    ) => {

        /// Global editor settings: the authoritative defaults for all
        /// configurable editor behaviour.
        ///
        /// [`EditorSettings::new`] is the single source of truth for these
        /// default values.
        #[derive(Clone)]
        pub struct EditorSettings {
            $( pub $gname: $gtype, )*
            $( pub $bname: $btype, )*
            $( pub $egname: $egtype, )*
            $( pub $ebname: $ebtype, )*
            $( $pname: $ptype, )*
            origins: SettingOrigins<TableKey>,
        }

        impl EditorSettings {
            /// Every setting at its default, for a run started in `$start`.
            pub fn new($start: &AbsolutePath) -> Self {
                Self {
                    $( $gname: $gdefault, )*
                    $( $bname: $bdefault, )*
                    $( $egname: $egdefault, )*
                    $( $ebname: $ebdefault, )*
                    $( $pname: <$ptype>::default(), )*
                    origins: SettingOrigins::default(),
                }
            }
        }

        impl EditorSettings {
            $(
                /// A `private_global` setting's value, written only by
                /// [`write_global`].
                pub fn $pname(&self) -> &$ptype {
                    &self.$pname
                }
            )*
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
            origins: SettingOrigins<BufferKey>,
        }

        impl BufferOverrides {
            $( buffer_accessor!($bname, $btype, $bparser); )*
        }

        // ── keys ──────────────────────────────────────────────────────────────

        /// A global-only setting: a `global` or `private_global` entry.
        /// Variants reuse the snake_case field names, as [`ResyncKey`]'s do.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[allow(non_camel_case_types)]
        pub(in crate::editor) enum GlobalKey {
            $( $gname, )*
            $( $pname, )*
        }

        impl GlobalKey {
            /// Every global-only setting.
            pub(in crate::editor) const ALL: &'static [Self] = &[$( Self::$gname, )* $( Self::$pname, )*];

            /// The key `:set` and scripts name the setting by.
            pub(in crate::editor) const fn name(self) -> &'static str {
                match self {
                    $( Self::$gname => $gkey, )*
                    $( Self::$pname => $pkey, )*
                }
            }

            /// The derived state a write to this setting resyncs, `None`
            /// unless its entry declares `resync: true`. The sole source both
            /// `editor::settings::ops::resync_derived_state` (which effect to
            /// run) and `reset_globals` (which keys need one at all) go
            /// through.
            pub(in crate::editor) fn resync(self) -> Option<ResyncKey> {
                match self {
                    $( $( Self::$gname if $gresync => Some(ResyncKey::$gname), )? )*
                    _ => None,
                }
            }
        }

        /// A setting a buffer can override: a `buffer` or `subfield` entry.
        /// Variants reuse the snake_case field names, as [`GlobalKey`]'s do.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[allow(non_camel_case_types)]
        pub(in crate::editor) enum BufferKey {
            $( $bname, )*
            $( $sfield, )*
        }

        impl BufferKey {
            /// Every setting a buffer can override.
            pub(in crate::editor) const ALL: &'static [Self] = &[$( Self::$bname, )* $( Self::$sfield, )*];

            /// The key `:set` and scripts name the setting by.
            pub(in crate::editor) const fn name(self) -> &'static str {
                match self {
                    $( Self::$bname => $bkey, )*
                    $( Self::$sfield => $skey, )*
                }
            }

            /// The [`PaneSetting`] the entry's `pane:` clause names, `None`
            /// for a key a pane cannot override.
            pub(in crate::editor) fn pane(self) -> Option<PaneSetting> {
                match self {
                    $( $( Self::$bname => Some(PaneSetting::$bpane), )? )*
                    _ => None,
                }
            }
        }

        /// The settings-table key a raw key names, by the section that
        /// declares it. `None` for any other key, notably the
        /// `BufferIntrinsicOption`s, which have no generic storage:
        /// [`SettingKey::named`] matches them before consulting this table.
        fn table_setting(key: &str) -> Option<TableKey> {
            match key {
                $( $gkey => Some(TableKey::Global(GlobalKey::$gname)), )*
                $( $pkey => Some(TableKey::Global(GlobalKey::$pname)), )*
                $( $bkey => Some(TableKey::Buffer(BufferKey::$bname)), )*
                $( $skey => Some(TableKey::Buffer(BufferKey::$sfield)), )*
                _ => None,
            }
        }

        // ── write_global / write_buffer ───────────────────────────────────────

        /// Write a global setting's raw value from `by`, with no
        /// derived-state resync.
        ///
        /// Returns `Err(message)` on an invalid value, checked before the
        /// rank, and `Ok(false)`, writing nothing, when a higher origin
        /// wrote the value ([`SettingOrigins`]).
        ///
        /// This is the raw field write only. Some settings have derived
        /// state that must be resynced after a successful write (declared
        /// via `resync: true` above). Production code must go through
        /// [`ops::apply_global`], which wraps this and runs those effects;
        /// calling this directly would skip them.
        ///
        /// [`ops::apply_global`] is the one production caller.
        /// `ops::write_global_for_test` reaches it for `testing::MockHost`
        /// and `settings::tests`, in test and `test-util` builds only.
        pub(in crate::editor::settings) fn write_global(
            key: TableKey,
            value: &OptionInput,
            settings: &mut EditorSettings,
            by: ConfigOrigin,
        ) -> Result<bool, String> {
            match key {
                $( TableKey::Global(GlobalKey::$gname) => {
                    let parsed = kind::parse_as::<kind::$gparser, _>($gkey, value)?;
                    if !settings.origins.claim(key, by) { return Ok(false); }
                    settings.$gname = parsed;
                } )*
                $( TableKey::Global(GlobalKey::$pname) => {
                    let parsed = kind::parse_as::<kind::$pparser, _>($pkey, value)?;
                    if !settings.origins.claim(key, by) { return Ok(false); }
                    settings.$pname = parsed;
                } )*
                $( TableKey::Buffer(BufferKey::$bname) => {
                    let parsed = kind::parse_as::<kind::$bparser, _>($bkey, value)?;
                    if !settings.origins.claim(key, by) { return Ok(false); }
                    settings.$bname = parsed;
                } )*
                $( TableKey::Buffer(BufferKey::$sfield) => {
                    let parsed = kind::parse_as::<kind::$sparser, _>($skey, value)?;
                    if !settings.origins.claim(key, by) { return Ok(false); }
                    settings.$sglobal.$ssub = parsed;
                } )*
            }
            Ok(true)
        }

        /// Write a buffer-scoped setting's raw override, with no derived-state
        /// resync (no buffer-scoped key has one today; see [`write_global`]'s
        /// doc for the mechanism global-only keys use).
        ///
        /// Returns `Err(message)` on an invalid value, and `Ok(false)`,
        /// writing nothing, when a higher origin wrote the override
        /// ([`SettingOrigins`]).
        ///
        /// `ops::apply_buffer`, behind [`ops::apply_buffer_target`], is the
        /// only production caller. `testing::MockHost` models no buffers, so
        /// it writes no override and needs no test entry point here, unlike
        /// [`write_global`].
        pub(in crate::editor::settings) fn write_buffer(
            key: BufferKey,
            value: &OptionInput,
            overrides: &mut BufferOverrides,
            by: ConfigOrigin,
        ) -> Result<bool, String> {
            match key {
                $( BufferKey::$bname => {
                    let parsed = kind::parse_as::<kind::$bparser, _>($bkey, value)?;
                    if !overrides.origins.claim(key, by) { return Ok(false); }
                    overrides.$bname = Some(parsed);
                } )*
                $( BufferKey::$sfield => {
                    let parsed = kind::parse_as::<kind::$sparser, _>($skey, value)?;
                    if !overrides.origins.claim(key, by) { return Ok(false); }
                    overrides.$sfield = Some(parsed);
                } )*
            }
            Ok(true)
        }

        // ── pane overrides ────────────────────────────────────────────────────

        /// A pane override's value, one variant per [`PaneSetting`].
        pub(in crate::editor) enum PaneValue {
            $( $( $bpane($btype), )? )*
        }

        /// Parse `value` for the pane override `setting`, by the parser its
        /// `buffer` entry declares.
        pub(in crate::editor) fn parse_pane(
            setting: PaneSetting,
            value: &OptionInput,
        ) -> Result<PaneValue, String> {
            match setting {
                $( $( PaneSetting::$bpane => kind::parse_as::<kind::$bparser, _>($bkey, value).map(PaneValue::$bpane), )? )*
            }
        }

        /// `value` as the text `:set pane` takes back.
        pub(in crate::editor) fn pane_text(value: PaneValue) -> String {
            match value {
                $( $( PaneValue::$bpane(value) => <kind::$bparser as kind::Kind<$btype>>::shape(&value).text(), )? )*
            }
        }

        // ── setting readers ───────────────────────────────────────────────────

        /// `key`'s global value.
        pub(in crate::editor) fn read_global(key: TableKey, settings: &EditorSettings) -> Shape {
            match key {
                $( TableKey::Global(GlobalKey::$gname) => <kind::$gparser as kind::Kind<$gtype>>::shape(&settings.$gname), )*
                $( TableKey::Global(GlobalKey::$pname) => <kind::$pparser as kind::Kind<$ptype>>::shape(&settings.$pname), )*
                $( TableKey::Buffer(BufferKey::$bname) => <kind::$bparser as kind::Kind<$btype>>::shape(&settings.$bname), )*
                $( TableKey::Buffer(BufferKey::$sfield) => <kind::$sparser as kind::Kind<$stype>>::shape(&settings.$sglobal.$ssub), )*
            }
        }

        /// `key`'s effective value for a buffer with `overrides`: the
        /// override if set, else the global value.
        pub(in crate::editor) fn read_buffer(
            key: BufferKey,
            settings: &EditorSettings,
            overrides: &BufferOverrides,
        ) -> Shape {
            match key {
                $( BufferKey::$bname => <kind::$bparser as kind::Kind<$btype>>::shape(overrides.$bname.as_ref().unwrap_or(&settings.$bname)), )*
                $( BufferKey::$sfield => <kind::$sparser as kind::Kind<$stype>>::shape(overrides.$sfield.as_ref().unwrap_or(&settings.$sglobal.$ssub)), )*
            }
        }

        impl TableKey {
            /// The values `:set` value completion offers for the setting,
            /// by the parser its entry declares. Empty for one that takes
            /// free text.
            pub(in crate::editor) fn value_names(self) -> &'static [&'static str] {
                match self {
                    $( Self::Global(GlobalKey::$gname) => <kind::$gparser as kind::Kind<$gtype>>::NAMES, )*
                    $( Self::Global(GlobalKey::$pname) => <kind::$pparser as kind::Kind<$ptype>>::NAMES, )*
                    $( Self::Buffer(BufferKey::$bname) => <kind::$bparser as kind::Kind<$btype>>::NAMES, )*
                    $( Self::Buffer(BufferKey::$sfield) => <kind::$sparser as kind::Kind<$stype>>::NAMES, )*
                }
            }
        }
    };
}

// A key names its area first (`indent-…`, `insert-…`, `lsp-…`), so `:set`'s
// prefix completion lists an area's settings together.
define_settings! {
    new(startup)
    global {
        "scroll-margin" => scroll_margin: usize = 3,
            parser: Usize;
        "scroll-jump-align" => scroll_jump_align: ObjectJumpAlign = ObjectJumpAlign::default(),
            parser: Typed;
        "insert-cursor-shape" => insert_cursor_shape: CursorShape = CursorShape::default(),
            parser: Typed;
        "mouse-scroll-lines" => mouse_scroll_lines: usize = 3,
            parser: Usize;
        "mouse" => mouse: bool = true,
            parser: Bool;
        "mouse-select" => mouse_select: bool = false,
            parser: Bool;
        // Resizes every open pane's live jump list cap. Like undo-levels
        // below, takes effect on the next push, not retroactively. See
        // `editor::settings::ops::resync_derived_state` and `JumpList::set_capacity`.
        "jump-list-capacity" => jump_list_capacity: usize = 100,
            parser: UsizeNonzero,
            resync: true;
        "jump-list-min-lines" => jump_list_min_lines: usize = 5,
            parser: Usize;
        // Resizes the command/search prompt-history ring cap. Like
        // undo-levels below, takes effect on the next push, not
        // retroactively. See `editor::settings::ops::resync_derived_state`
        // and `History::set_capacity`.
        "prompt-history-levels" => prompt_history_levels: usize = 100,
            parser: UsizeNonzero,
            resync: true;
        // 0 is a valid, meaningful value here (unlimited), unlike
        // prompt-history-levels above, hence `Usize`, not `UsizeNonzero`.
        // Resizes the undo-tree cap on every open buffer. Takes effect on
        // the next edit, not retroactively (Vim's `undolevels` semantics).
        // See `editor::settings::ops::resync_derived_state`.
        "undo-levels" => undo_levels: usize = 0,
            parser: Usize,
            resync: true;
        // Where a save writes the undo history of a buffer whose
        // `undo-persist` is on, and where opening a file looks for one.
        "undo-directory" => undo_directory: StoreDirectory = StoreDirectory::Default,
            parser: Typed;
        // Whether a run keeping a session (started with no files, or after
        // `:save-session`) restores the one saved for `session-root` and
        // saves its own when it ends.
        "session-persist" => session_persist: bool = false,
            parser: Bool;
        // Where sessions are saved and looked for.
        "session-directory" => session_directory: StoreDirectory = StoreDirectory::Default,
            parser: Typed;
        // The directory a session belongs to, whatever the working directory.
        "session-root" => session_root: SessionRoot = SessionRoot::Path(startup.clone()),
            parser: Typed;
        "steel-init-budget-ms" => steel_init_budget_ms: usize = 10_000,
            parser: UsizeNonzero;
        "steel-command-budget-ms" => steel_command_budget_ms: usize = 1_000,
            parser: UsizeNonzero;
        // How long a write waits for `on-before-buffer-save` handlers that
        // took `done` before it goes ahead without them.
        "hook-before-save-timeout-ms" => hook_before_save_timeout_ms: usize = 1_000,
            parser: UsizeNonzero;
        "command-completion" => command_completion: CommandCompletion = CommandCompletion::default(),
            parser: Typed;
        "popup-border" => popup_border: bool = true,
            parser: Bool;
        // Read by the completion menu's documentation box and by the
        // selection-time `completionItem/resolve` that fills it.
        "completion-docs" => completion_docs: bool = true,
            parser: Bool;
        "pane-dividers" => pane_dividers: bool = true,
            parser: Bool;
        // Read fresh by the statusline provider each frame; no resync needed.
        "statusline-mode-colors" => statusline_mode_colors: bool = true,
            parser: Bool;
        // Resolved into the tab bar's rows by `EditorState::screen_layout`, and
        // into the `TablineViewState` snapshot the provider paints by
        // `sync_tabline_view` every frame.
        "tabline" => tabline: TablineVisibility = TablineVisibility::default(),
            parser: Typed;
        // Loads and applies the named theme immediately, rolling back to the
        // theme on screen on failure; empty selects the compiled-in fallback.
        // See `editor::settings::ops::resync_derived_state`.
        "theme" => theme: String = DEFAULT_THEME.to_string(),
            parser: FreeText,
            resync: true;
        // The program `pipe` runs each command line with.
        "shell" => shell: ShellCommand = ShellCommand::default(),
            parser: Shell;
        "syntax-highlight-max-bytes" => syntax_highlight_max_bytes: usize = 1_048_576,
            parser: UsizeNonzero;
        // Scroll bursts (page-down held, mouse wheel) must collapse to one
        // OnViewportChange fire, not one per frame.
        "hook-viewport-debounce-ms" => hook_viewport_debounce_ms: usize = 150,
            parser: UsizeNonzero;
        // rust-analyzer's first requests during indexing are slow; 10s
        // gives real-world servers room before the request is dropped as
        // TimedOut.
        "lsp-request-timeout-ms" => lsp_request_timeout_ms: usize = 10_000,
            parser: UsizeNonzero;
        // A typing burst collapses to one diagnostics pull per buffer.
        "lsp-diagnostics-pull-debounce-ms" => lsp_diagnostics_pull_debounce_ms: usize = 250,
            parser: UsizeNonzero;
        // Hint = most lenient: every severity renders. Gates the diagnostic
        // underline/extra-highlight and gutter-sign render write sides.
        "lsp-diagnostics-min-severity" => lsp_diagnostics_min_severity: DiagSeverity = DiagSeverity::Hint,
            parser: Typed;
        // Off hides the diagnostic underline and end-of-line summary on the
        // primary cursor's line while typing in Insert mode. The summary is
        // hidden by `core:lsp` passing the negation to `set-eol-text!`.
        "lsp-diagnostics-on-insert-line" => lsp_diagnostics_on_insert_line: bool = false,
            parser: Bool;
        "messages-min-severity" => messages_min_severity: MessagesMinSeverity = MessagesMinSeverity::default(),
            parser: Typed;
    }
    buffer {
        // A buffer-overridable setting like any other (e.g. from an
        // `on-language-set` hook, for a per-filetype default), plus a third,
        // narrower rung: `pane:` below adds `Scope::Pane`, and
        // `:set pane wrap-mode=…` (see `typed_settings::typed_set`) writes
        // straight to the live `Pane`'s override, a separate path from
        // `write_global`/`write_buffer`, since wrap is also a view property
        // (two panes on the same buffer may wrap differently once one is
        // pinned; see `commands::effective_wrap_mode`, the pane → buffer →
        // global resolver every render/motion path reads through).
        "wrap-mode" => wrap_mode: WrapMode = hume_engine::pane::DEFAULT_WRAP_STYLE,
            parser: Typed,
            pane: WrapMode;
        "indent-width" => indent_width: u8 = 4,
            parser: TabWidth;
        "indent-guides" => indent_guides: bool = true,
            parser: Bool;
        "indent-style" => indent_style: TabStyle = TabStyle::default(),
            parser: Typed;
        "gutter-line-numbers" => gutter_line_numbers: LineNumberStyle = LineNumberStyle::default(),
            parser: Typed;
        // The fold carets beside the text (`FoldColumn`), shown while the
        // buffer's grammar names foldable regions or the pane has a fold
        // closed.
        "gutter-folds" => gutter_folds: bool = true,
            parser: Bool;
        "insert-auto-pairs" => insert_auto_pairs: bool = true,
            parser: Bool;
        // Enter, `o` and `O` indent a new line (see `break_line`), and a
        // typed closing token dedents its line (`electric_dedent`). Off, a
        // new line starts at the left margin and no line is dedented.
        "indent-smart" => indent_smart: bool = true,
            parser: Bool;
        // Leaving Insert mode selects whatever text the session just typed
        // (empty run: falls back to the entry command's own exit position).
        // See `begin_typed_run` and `tear_down_insert`'s pinned-anchor
        // finalization. Applies to every way of entering Insert mode:
        // `i`/`a`/`I`/`A`/`o`/`O`/`c`.
        "insert-select-on-exit" => insert_select_on_exit: bool = true,
            parser: Bool;
        // `toggle-comment` on a selection spanning several lines takes a
        // line token per line instead of the language's block pair.
        "comment-prefer-line" => comment_prefer_line: bool = false,
            parser: Bool;
        // Scope header rows over a pane's top rows (`hume_engine::scope_headers`).
        "context-enabled" => context_enabled: bool = true,
            parser: Bool;
        // With more scopes than header rows, keep the outermost one.
        "context-outer-level" => context_outer_level: bool = false,
            parser: Bool;
        // Word motions (`w`/`W`/`b`/`B`), `mm`/`MM` and `maw`/`maW` cover
        // the destination word's whitespace bookend on the preferred side.
        // See `word_select_cmd`'s `ctx.whitespace` read and `run_body`'s
        // `SelectionBody::Word` arm, which resolves it.
        "word-selects-whitespace" => word_selects_whitespace: WordSelectsWhitespace = WordSelectsWhitespace::default(),
            parser: Typed;
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
            parser: WordChars;
        "gutter-signs" => gutter_signs: SignColumnConfig = SignColumnConfig::default(),
            parser: Typed;
        // Read fresh by `check_buffer_disk_state` at each trigger; no
        // resync needed. `true`: an external change to the focused buffer
        // opens a reload confirm. `false`: detection still runs and warns,
        // but reload stays manual via `:e!`/`:checktime`. Independent of
        // `:w`'s write guard, which reads the file itself at write time
        // regardless of this setting.
        "file-reload-prompt" => file_reload_prompt: bool = true,
            parser: Bool;
        "undo-persist" => undo_persist: bool = false,
            parser: Bool;
    }
    extra_global {
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
            parser: Typed;
        "whitespace-tab" => whitespace.tab / whitespace_tab : WhitespaceRender,
            parser: Typed;
        "whitespace-newline" => whitespace.newline / whitespace_newline : bool,
            parser: ShowNewline;
    }
    private_global {
        "statusline" => statusline: StatusLineConfig, parser: Statusline;
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

/// Applying a setting change: the single production path
/// ([`ops::apply_global`]/[`ops::apply_buffer_target`]). A child of this module,
/// not a sibling, so [`write_global`]/[`write_buffer`] above narrow to
/// `pub(in crate::editor::settings)`: the chokepoint this crate's write path
/// funnels through is reachable from exactly `settings::ops` and
/// `settings::tests`, not from every one of the ~110 other files under
/// `crate::editor`. The module path itself is `pub(crate)`, wider than that,
/// only so `testing::mock_host` (outside `crate::editor` entirely) can
/// name `ops::write_global_for_test` and `ops::global_option`; every other item in `ops` keeps its
/// own narrower per-item visibility regardless of the path being nameable.
pub(crate) mod ops;

mod kind;
mod setting_arg;
mod values;
mod word_list;

pub(in crate::editor) use kind::Shape;
pub(in crate::editor) use setting_arg::{ArgSplit, SettingArg};

pub use values::{
    CommandCompletion, CursorShape, MessagesMinSeverity, ObjectJumpAlign, SessionRoot,
    SignColumnConfig, SignColumnMode, StoreDirectory, TablineVisibility, WordSelectsWhitespace,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod manual_options_drift;
