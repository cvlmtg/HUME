//! Global settings, statusline config, and the Steel eval budget.

use hume_engine::pipeline::BufferId;

/// A setting's effective value, typed just enough for `(get-option key)` to
/// build the right `SteelVal`. `hume-scripting` has no dependency on
/// `hume-editor`'s settings types, so the editor impl converts its own
/// per-key parser kind (`bool`/`usize`/`from_str`/…) down to one of these
/// three shapes at the trait boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

/// Global settings, statusline config, and the Steel eval budget,
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

    /// Init-only; the editor parses element names into `StatusElement`.
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String>;

    /// Steel eval budget in milliseconds for command / hook execution.
    fn steel_command_budget_ms(&self) -> u64;
}
