//! `EditorHostImpl`'s global settings, statusline config, and the Steel
//! eval budget.

use hume_core::{BufferId, OptionInput};

use super::EditorHostImpl;
use crate::editor::buffer::Buffer;
use crate::editor::settings::SettingText;
use crate::editor::settings::ops;
use hume_scripting::host::{OptionValue, SettingsHost};

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
        ops::apply_global(self.state, self.view, key, &value, origin)
    }

    fn set_buffer_option(
        &mut self,
        key: &str,
        value: OptionInput,
        bid: BufferId,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String> {
        self.live_buffer(bid)?;
        ops::apply_buffer(self.state, bid, key, &value, origin)
    }

    fn set_buffer_line_ending(
        &mut self,
        bid: BufferId,
        value: OptionInput,
        origin: hume_core::ConfigOrigin,
    ) -> Result<(), String> {
        self.live_buffer(bid)?;
        ops::set_line_ending(self.state, bid, &value, origin)
    }

    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        crate::editor::settings::setting_value(key, &self.state.settings, None)
            .ok_or_else(|| format!("unknown setting '{key}'"))
    }

    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        let buf = self.live_buffer(bid)?;
        crate::editor::settings::setting_value(key, &self.state.settings, Some(&buf.overrides))
            .ok_or_else(|| format!("unknown setting '{key}'"))
    }

    fn buffer_line_ending(&self, bid: BufferId) -> Result<String, String> {
        Ok(self.live_buffer(bid)?.text().line_ending().to_text())
    }

    fn steel_command_budget_ms(&self) -> u64 {
        self.state.settings.steel_command_budget_ms as u64
    }
}
