//! Global settings and the Steel eval budget.

use hume_engine::pipeline::BufferId;

/// A buffer option stored on the buffer itself rather than in the settings
/// tables. Each has no global default and is valid only for `:set buffer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferIntrinsicOption {
    /// The buffer's language. `set-buffer-option!`/`get-buffer-option` handle
    /// it themselves: a change is queued as `Effect::SetBufferLanguage`, and a
    /// read has to see one queued earlier in the same eval.
    Language,
    /// The line ending a save writes (`lf` or `crlf`).
    LineEnding,
}

/// Generate `BufferIntrinsicOption`'s `ALL` and `key` from one list of
/// `Variant => "key"` pairs. `key` is an exhaustive match, so a variant left
/// out of the list fails to compile instead of going missing from `ALL`.
macro_rules! buffer_intrinsic_options {
    ($($variant:ident => $key:literal),+ $(,)?) => {
        impl BufferIntrinsicOption {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The option name as `:set` and the scripting API spell it.
            pub const fn key(self) -> &'static str {
                match self {
                    $(Self::$variant => $key,)+
                }
            }
        }
    };
}

buffer_intrinsic_options! {
    Language => "language",
    LineEnding => "line-ending",
}

impl BufferIntrinsicOption {
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|opt| opt.key() == key)
    }
}

/// A `language` option value as a language name: `""` is the spelling of
/// "no language", so it decodes to `None`.
pub fn language_option_value(value: &str) -> Option<&str> {
    Some(value).filter(|name| !name.is_empty())
}

/// A setting's effective value, typed just enough for `(get-option key)` to
/// build the right `SteelVal`. `hume-scripting` has no dependency on
/// `hume-editor`'s settings types, so the editor impl converts its own
/// per-key parser kind (`bool`/`usize`/`enum_str`/…) down to one of these
/// four shapes at the trait boundary. `Symbol` is a closed set of names
/// (`tab-style`'s `hard`/`soft`), `Str` free text or a `name:N` form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Str(String),
    Symbol(String),
}

impl From<OptionValue> for steel::rvals::SteelVal {
    fn from(value: OptionValue) -> Self {
        match value {
            OptionValue::Bool(b) => Self::BoolV(b),
            OptionValue::Int(n) => Self::IntV(n as isize),
            OptionValue::Str(s) => Self::StringV(s.into()),
            OptionValue::Symbol(s) => Self::SymbolV(s.into()),
        }
    }
}

/// Global settings and the Steel eval budget,
/// accessed through [`EditorHost::settings`](super::EditorHost::settings).
pub trait SettingsHost {
    /// `(set-option! key value)`: only `Global` scope from scripts. No
    /// eval-mode gate (`open` kind): callable from `init.scm`, plugin load,
    /// plugin activation, or a plain command/hook body. The write already
    /// goes through the editor's validating chokepoint regardless of caller.
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String>;

    /// `(set-buffer-option! pane key value)`: writes `key`'s per-buffer
    /// override on `pane`'s buffer. Command/hook context (`cmd` kind),
    /// unlike `set_global_option`. `Err` for a stale buffer, a global-only
    /// key, or a bad value. Kind-C: only the buffer matters, `pane`'s own
    /// pane (if any) is ignored: the builtin layer narrows the decoded
    /// `PaneHandle` to a plain `bid` before this is called.
    fn set_buffer_option(&mut self, key: &str, value: &str, bid: BufferId) -> Result<(), String>;

    /// `(get-option key)`: `key`'s global value, ignoring any buffer
    /// override even if one exists. `Err` for an unknown key. No eval-mode
    /// gate (`open` kind), mirroring `set_global_option`: callable from
    /// `init.scm` too.
    fn get_global_option(&self, key: &str) -> Result<OptionValue, String>;

    /// `(get-buffer-option pane key)`: the effective value of `key` for
    /// `pane`'s buffer: its buffer override if one is set, else the global
    /// default. `Err` for an unknown key or a stale buffer, same guard as
    /// `set_buffer_option`; a closed buffer's override no longer exists to
    /// read, so silently falling back to the global value would hide a
    /// caller acting on a buffer that already went away. Command/hook
    /// context (`cmd` kind): the idiomatic caller is a hook handler that
    /// received the target pane as an explicit argument (e.g.
    /// `on-language-set`, whose buffer may differ from the focused one).
    /// Kind-C, same as `set_buffer_option`.
    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String>;

    /// Steel eval budget in milliseconds for command / hook execution.
    fn steel_command_budget_ms(&self) -> u64;
}
