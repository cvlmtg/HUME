//! `EditorHostImpl`'s global settings, statusline config, and the Steel
//! eval budget.

use hume_core::{BufferId, BufferOptionWrite, OptionInput, OptionValue};

use super::EditorHostImpl;
use crate::editor::buffer::Buffer;
use crate::editor::settings::{OptionAccess, buffer_target, global_key, ops, read_global};
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

impl<'a> SettingsHost for EditorHostImpl<'a> {
    fn set_global_option(
        &mut self,
        key: &str,
        value: OptionInput,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String> {
        let key = global_key(key, OptionAccess::Write)?;
        ops::apply_global(self.state, self.view, key, &value, origin)
    }

    fn set_buffer_option(
        &mut self,
        key: &str,
        value: OptionInput,
        bid: BufferId,
        origin: hume_core::ConfigOrigin,
    ) -> Result<BufferOptionWrite, String> {
        let target = buffer_target(key, OptionAccess::Write)?;
        self.live_buffer(bid)?;
        ops::apply_buffer_target(self.state, bid, target, &value, origin)
    }

    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        let key = global_key(key, OptionAccess::Read)?;
        Ok(read_global(key, &self.state.settings).value())
    }

    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        let target = buffer_target(key, OptionAccess::Read)?;
        let buf = self.live_buffer(bid)?;
        Ok(ops::read_buffer_target(self.state, buf, target).value())
    }

    fn steel_command_budget_ms(&self) -> u64 {
        self.state.settings.steel_command_budget_ms as u64
    }
}
