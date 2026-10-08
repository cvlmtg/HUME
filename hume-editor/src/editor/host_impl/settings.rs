//! `EditorHostImpl`'s global settings, statusline config, and the Steel
//! eval budget.

use hume_engine::pipeline::BufferId;

use super::EditorHostImpl;
use hume_scripting::host::{LINE_ENDING_OPTION, OptionValue, SettingsHost};

impl<'a> SettingsHost for EditorHostImpl<'a> {
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        crate::editor::settings::ops::apply_global(self.state, self.view, key, value)
    }

    fn set_buffer_option(&mut self, key: &str, value: &str, bid: BufferId) -> Result<(), String> {
        // `settings::ops::apply_buffer`'s `get_mut` panics on a stale id;
        // validate first so a bad `bid` from Steel becomes an `Err`, not a
        // panic.
        if self.state.buffers.try_get(bid).is_none() {
            return Err(format!("set-buffer-option!: invalid buffer id {bid:?}"));
        }
        crate::editor::settings::ops::apply_buffer(self.state, bid, key, value)
    }

    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        crate::editor::settings::setting_value(key, &self.state.settings, None)
            .ok_or_else(|| format!("get-option: unknown setting '{key}'"))
    }

    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        // Same `try_get` guard as `set_buffer_option` above: a stale `bid`
        // is invalid input, not a request to fall back to the global value.
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return Err(format!("get-buffer-option: invalid buffer id {bid:?}"));
        };
        if key == LINE_ENDING_OPTION {
            return Ok(OptionValue::Symbol(
                buf.text().line_ending().as_str().to_string(),
            ));
        }
        crate::editor::settings::setting_value(key, &self.state.settings, Some(&buf.overrides))
            .ok_or_else(|| format!("get-buffer-option: unknown setting '{key}'"))
    }

    fn steel_command_budget_ms(&self) -> u64 {
        self.state.settings.steel_command_budget_ms as u64
    }
}
