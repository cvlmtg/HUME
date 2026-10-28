//! Global settings and the Steel eval budget.

use hume_core::{BufferId, OptionInput, StatuslineSections};

/// A setting's effective value, typed just enough for `(get-option key)` to
/// build the right `SteelVal`. `hume-scripting` has no dependency on
/// `hume-editor`'s settings types, so the editor impl converts its own
/// per-key parser kind (`bool`/`usize`/`enum_str`/…) down to one of these
/// shapes at the trait boundary. `Str` is every named value, free text
/// and `name:N` forms alike, in the spelling `set-option!` takes back;
/// `List` is a word list (`shell`); `Sections` is the `statusline`, read
/// back as `(hash 'left 'center 'right)`, the hash `set-option!` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<String>),
    Sections(StatuslineSections),
}

impl From<OptionValue> for steel::rvals::SteelVal {
    fn from(value: OptionValue) -> Self {
        match value {
            OptionValue::Bool(b) => Self::BoolV(b),
            OptionValue::Int(n) => Self::IntV(n as isize),
            OptionValue::Str(s) => Self::StringV(s.into()),
            OptionValue::List(words) => crate::builtins::args::string_list(words),
            OptionValue::Sections(sections) => crate::builtins::settings::sections_value(sections),
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
    /// It ranks as `origin`: a value a higher origin set stays, and the
    /// write is reported at Trace instead.
    fn set_global_option(
        &mut self,
        key: &str,
        value: OptionInput,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String>;

    /// `(set-buffer-option! pane key value)`: writes `key`'s per-buffer
    /// override on `pane`'s buffer. `key` is a settings-table key: the
    /// builtins route each `BufferIntrinsicOption` elsewhere. Command/hook
    /// context (`cmd` kind),
    /// unlike `set_global_option`. `Err` for a stale buffer, a global-only
    /// key, or a bad value. Only the buffer matters: the builtin declares
    /// its `pane` a `LiveBuffer`, which resolves to a plain `bid`. It ranks as
    /// `origin` against the buffer's override, as `set_global_option` does
    /// against the global value.
    fn set_buffer_option(
        &mut self,
        key: &str,
        value: OptionInput,
        bid: BufferId,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String>;

    /// `(set-buffer-option! pane "line-ending" value)`: sets the line ending
    /// a save writes on `bid`, ranked as `origin`. `Err` for a stale buffer,
    /// a read-only buffer, or a value other than `lf`/`crlf`.
    fn set_buffer_line_ending(
        &mut self,
        bid: BufferId,
        value: OptionInput,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String>;

    /// `(get-option key)`: `key`'s global value, ignoring any buffer
    /// override even if one exists. `Err` for an unknown key. No eval-mode
    /// gate (`open` kind), mirroring `set_global_option`: callable from
    /// `init.scm` too.
    fn get_global_option(&self, key: &str) -> Result<OptionValue, String>;

    /// `(get-buffer-option pane key)`: the effective value of `key` for
    /// `pane`'s buffer: its buffer override if one is set, else the global
    /// default. `key` is a settings-table key, as for `set_buffer_option`.
    /// `Err` for an unknown key or a stale buffer, same guard as
    /// `set_buffer_option`; a closed buffer's override no longer exists to
    /// read, so silently falling back to the global value would hide a
    /// caller acting on a buffer that already went away. Command/hook
    /// context (`cmd` kind): the idiomatic caller is a hook handler that
    /// received the target pane as an explicit argument (e.g.
    /// `on-language-set`, whose buffer may differ from the focused one).
    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String>;

    /// `(get-buffer-option pane "line-ending")`: in the spelling the setter
    /// takes back. `Err` for a stale buffer.
    fn buffer_line_ending(&self, bid: BufferId) -> Result<String, String>;

    /// Steel eval budget in milliseconds for command / hook execution.
    fn steel_command_budget_ms(&self) -> u64;
}
