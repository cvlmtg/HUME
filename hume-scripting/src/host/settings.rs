//! Global settings and the Steel eval budget.

use hume_core::{BufferId, OptionInput, OptionValue};

/// `value` as a script reads it back: a word list as a list of strings, the
/// statusline as `(hash 'left 'center 'right)`, the hash `set-option!` takes.
pub fn option_value_to_steel(value: OptionValue) -> steel::rvals::SteelVal {
    use steel::rvals::SteelVal;
    match value {
        OptionValue::Bool(b) => SteelVal::BoolV(b),
        OptionValue::Int(n) => SteelVal::IntV(n as isize),
        OptionValue::Str(s) => SteelVal::StringV(s.into()),
        OptionValue::List(words) => crate::builtins::args::string_list(words),
        OptionValue::Sections(sections) => crate::builtins::settings::sections_value(sections),
    }
}

/// Global settings and the Steel eval budget,
/// accessed through [`EditorHost::settings`](super::EditorHost::settings).
///
/// A key is the name a script passed. The host knows which options there
/// are, which scopes each takes and what value each takes, and refuses a
/// key or a value with an error naming the builtin to use instead.
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

    /// `(set-buffer-option! pane key value)`: writes `key` on `pane`'s
    /// buffer: its per-buffer override, or for `line-ending` the line ending
    /// a save writes. Every buffer option but `language`, whose write the
    /// builtin queues. Command/hook context (`cmd` kind), unlike
    /// `set_global_option`. `Err` for a stale buffer, a bad value, or a
    /// read-only buffer's line ending. Only the buffer matters: the builtin
    /// declares its `pane` a `LiveBuffer`, which resolves to a plain `bid`.
    /// It ranks as `origin` against the buffer's own value, as
    /// `set_global_option` does against the global value.
    fn set_buffer_option(
        &mut self,
        key: &str,
        value: OptionInput,
        bid: BufferId,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String>;

    /// `(get-option key)`: `key`'s global value, ignoring any buffer
    /// override even if one exists. No eval-mode gate (`open` kind),
    /// mirroring `set_global_option`: callable from `init.scm` too.
    fn get_global_option(&self, key: &str) -> Result<OptionValue, String>;

    /// `(get-buffer-option pane key)`: the effective value of `key` for
    /// `pane`'s buffer: its buffer override if one is set, else the global
    /// default, or the value the buffer stores itself for `line-ending`.
    /// `Err` for a stale buffer, same guard as `set_buffer_option`: a closed
    /// buffer has no override to read, and falling back to the global value
    /// would hide a caller acting on a buffer that went away. Command/hook context (`cmd` kind): the idiomatic
    /// caller is a hook handler that received the target pane as an explicit
    /// argument (e.g. `on-language-set`, whose buffer may differ from the
    /// focused one).
    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String>;

    /// Steel eval budget in milliseconds for command / hook execution.
    fn steel_command_budget_ms(&self) -> u64;
}
