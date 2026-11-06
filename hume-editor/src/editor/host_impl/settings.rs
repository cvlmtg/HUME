//! `EditorHostImpl`'s global settings, statusline config, and the Steel
//! eval budget.

use hume_core::{BufferId, OptionInput, OptionValue};

use super::EditorHostImpl;
use crate::editor::buffer::Buffer;
use crate::editor::settings::{
    BufferTarget, SettingKey, TableKey, ops, read_global, unknown_setting,
};
use hume_scripting::host::SettingsHost;

impl EditorHostImpl<'_> {
    /// `bid`'s buffer, or `Err` for a stale id. The writers this guards
    /// panic on a stale id, and a read must not fall back to the global
    /// value for a buffer that went away.
    fn live_buffer(&self, bid: BufferId) -> Result<&Buffer, String> {
        self.buffer(bid)
            .ok_or_else(|| format!("invalid buffer id {bid:?}"))
    }
}

/// The setting a script's `key` names. `Err` for an unknown key.
fn setting(key: &str) -> Result<SettingKey, String> {
    SettingKey::named(key).ok_or_else(|| unknown_setting(key))
}

/// The global value a global builtin's `key` names. An option the buffer
/// stores itself has none, and its refusal names `buffer_builtin`.
fn global_key(key: &str, buffer_builtin: &str) -> Result<TableKey, String> {
    setting(key)?
        .global()
        .ok_or_else(|| format!("'{key}' is per-buffer: use {buffer_builtin}"))
}

/// What a buffer builtin's `key` names on a buffer. A global-only option
/// has nothing there, and its refusal names `global_builtin`.
fn buffer_target(key: &str, global_builtin: &str) -> Result<BufferTarget, String> {
    setting(key)?
        .buffer()
        .ok_or_else(|| format!("'{key}' is global-only: use {global_builtin}"))
}

impl<'a> SettingsHost for EditorHostImpl<'a> {
    fn set_global_option(
        &mut self,
        key: &str,
        value: OptionInput,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String> {
        let key = global_key(key, "set-buffer-option!")?;
        ops::apply_global(self.state, self.view, key, &value, origin)
    }

    fn set_buffer_option(
        &mut self,
        key: &str,
        value: OptionInput,
        bid: BufferId,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String> {
        let target = buffer_target(key, "set-option!")?;
        self.live_buffer(bid)?;
        ops::apply_buffer_target(self.state, bid, target, &value, origin)
    }

    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        let key = global_key(key, "get-buffer-option")?;
        Ok(read_global(key, &self.state.settings).value())
    }

    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        let target = buffer_target(key, "get-option")?;
        let buf = self.live_buffer(bid)?;
        Ok(ops::read_buffer_target(self.state, buf, target).value())
    }

    fn steel_command_budget_ms(&self) -> u64 {
        self.state.settings.steel_command_budget_ms as u64
    }
}
