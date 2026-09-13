//! `SettingsHost` — moved out of `host_impl.rs`'s per-capability split.

use hume_engine::pipeline::BufferId;

use crate::statusline::StatusLineConfig;

use super::EditorHostImpl;
use hume_scripting::host::{OptionValue, SettingsHost};

impl<'a> SettingsHost for EditorHostImpl<'a> {
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        crate::editor::settings::ops::apply_global(self.state, self.view, key, value)
    }

    fn set_buffer_option(&mut self, key: &str, value: &str, bid: BufferId) -> Result<(), String> {
        // `settings::ops::apply_buffer`'s `get_mut` panics on a stale id —
        // validate first so a bad `bid` from Steel becomes an `Err`, not a
        // panic.
        if self.state.buffers.try_get(bid).is_none() {
            return Err(format!("set-buffer-option!: invalid buffer id {bid:?}"));
        }
        crate::editor::settings::ops::apply_buffer(self.state, bid, key, value)
    }

    fn get_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        let overrides = self.state.buffers.try_get(bid).map(|b| &b.overrides);
        crate::editor::settings::setting_value(key, &self.state.settings, overrides)
            .ok_or_else(|| format!("get-option: unknown setting '{key}'"))
    }

    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String> {
        // Validate here (for a section-labeled error message), then hand the
        // re-serialized wire string to the chokepoint so the write itself goes
        // through `write_global` like every other setting — see
        // `settings::ops::apply_global`'s doc for why a raw field write must
        // not bypass it.
        let cfg = StatusLineConfig {
            left: crate::statusline::parse_statusline_section(left, "left")?,
            center: crate::statusline::parse_statusline_section(center, "center")?,
            right: crate::statusline::parse_statusline_section(right, "right")?,
        };
        let wire = crate::editor::settings::format_statusline(&cfg);

        crate::editor::settings::ops::apply_global(self.state, self.view, "statusline", &wire)
    }

    fn steel_command_budget_ms(&self) -> u64 {
        self.state.settings.steel_command_budget_ms as u64
    }
}
