//! Global settings, statusline config, and the Steel eval budget — moved
//! out of `host.rs`'s per-capability split.

use hume_engine::pipeline::BufferId;

/// A setting's effective value, typed just enough for `(get-option key)` to
/// build the right `SteelVal` — `hume-scripting` has no dependency on
/// `hume-editor`'s settings types, so the editor impl converts its own
/// per-key parser kind (`bool`/`usize`/`from_str`/…) down to one of these
/// three shapes at the trait boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

/// Global settings, statusline config, and the Steel eval budget —
/// accessed through [`EditorHost::settings`](super::EditorHost::settings).
pub trait SettingsHost {
    /// `(set-option! key value)` — only `Global` scope from scripts. No
    /// eval-mode gate (`open` kind): callable from `init.scm`, plugin load,
    /// plugin activation, or a plain command/hook body — the write already
    /// goes through the editor's validating chokepoint regardless of caller.
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String>;

    /// `(set-buffer-option! bid key value)` — writes `key`'s per-buffer
    /// override on `bid`. Command/hook context (`cmd` kind), unlike
    /// `set_global_option`. `Err` for a stale `bid`, a global-only key, or a
    /// bad value.
    fn set_buffer_option(&mut self, key: &str, value: &str, bid: BufferId) -> Result<(), String>;

    /// `(get-option [bid] key)` — the effective value of `key`:
    /// `bid`'s buffer override if one is set, else the global default. `Err`
    /// for an unknown key. No eval-mode gate (`open` kind): callable from
    /// `init.scm` too — a stale or default `bid` degrades gracefully to the
    /// global default rather than erroring.
    fn get_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String>;

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
