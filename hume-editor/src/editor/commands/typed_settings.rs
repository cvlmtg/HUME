//! `:set` and `:get`: the typed commands that name a setting by scope and key.

use hume_core::{BufferOptionWrite, OptionInput, Severity};

use super::super::Editor;
use super::FocusedPane;
use crate::editor::error::CommandError;
use crate::editor::registry::ArgError;
use crate::editor::settings::ops as settings_ops;
use crate::editor::settings::{
    PaneSetting, PaneValue, Scope, SettingArg, SettingKey, SettingTarget, pane_text, parse_pane,
    read_global, unknown_setting,
};

/// The typed command that names a setting by scope and key.
#[derive(Clone, Copy)]
enum SettingCommand {
    Set,
    Get,
}

impl SettingCommand {
    /// What the command does to a setting, as its errors say it.
    fn action(self) -> &'static str {
        match self {
            Self::Set => "set with :set",
            Self::Get => "read with :get",
        }
    }
}

/// Resolve `key` in the scope `scope_str` names. The key is checked before
/// the scope, so an invalid scope on a real key names that key's scopes.
fn resolve_setting(
    cmd: SettingCommand,
    scope_str: &str,
    key: &str,
) -> Result<SettingTarget, CommandError> {
    let setting =
        SettingKey::named(key).ok_or_else(|| CommandError::transient(unknown_setting(key)))?;
    let target = Scope::named(scope_str).and_then(|scope| setting.target(scope));
    target.ok_or_else(|| {
        CommandError::transient(format!(
            "'{key}' cannot be {} {scope_str} (valid scopes: {})",
            cmd.action(),
            setting
                .scopes()
                .map(Scope::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

pub(in crate::editor) fn typed_set(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: &str,
) -> Result<(), ArgError> {
    let SettingArg::Value { scope, key, value } = SettingArg::parse(arg) else {
        return Err(ArgError::Usage);
    };
    let bid = fp.bid(&ed.view);
    // Only the user runs a typed command: no script reaches `:set`.
    let by = hume_core::ConfigOrigin::User;
    let input = OptionInput::Text(value.to_string());

    match resolve_setting(SettingCommand::Set, scope, key)? {
        SettingTarget::Global(key) => {
            settings_ops::apply_global(&mut ed.state, &mut ed.view, key, &input, by)
        }
        SettingTarget::Buffer(target) => {
            settings_ops::apply_buffer_target(&mut ed.state, bid, target, &input, by).map(
                |written| match written {
                    BufferOptionWrite::Applied => {}
                    BufferOptionWrite::Language(name) => {
                        let _ = ed.set_buffer_language_by_name(bid, name.as_deref(), by);
                    }
                },
            )
        }
        SettingTarget::Pane(setting) => parse_pane(setting, &input).map(|value| match value {
            PaneValue::WrapMode(mode) => ed.set_wrap_override(fp, mode),
        }),
    }
    .map_err(|e| ArgError::Failed(CommandError::new(e)))
}

/// Report a setting's effective value in `scope` as `key=value`, the text
/// `:set` takes back.
pub(in crate::editor) fn typed_get(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: &str,
) -> Result<(), ArgError> {
    let SettingArg::Key { scope, key } = SettingArg::parse(arg) else {
        return Err(ArgError::Usage);
    };
    let bid = fp.bid(&ed.view);
    let buf = ed.state.buffers.get(bid);
    let settings = &ed.state.settings;
    let value = match resolve_setting(SettingCommand::Get, scope, key)? {
        SettingTarget::Global(key) => read_global(key, settings).text(),
        SettingTarget::Buffer(target) => {
            settings_ops::read_buffer_target(&ed.state, buf, target).text()
        }
        SettingTarget::Pane(PaneSetting::WrapMode) => pane_text(PaneValue::WrapMode(
            super::effective_wrap_mode(buf, settings, &ed.view.panes[fp.pid()]),
        )),
    };
    ed.report(Severity::Info, format!("{key}={value}"));
    Ok(())
}
