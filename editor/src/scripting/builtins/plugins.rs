//! Plugin lifecycle builtins: `%declare-plugin!`, `push-current-plugin!`,
//! `pop-current-plugin!`, `resolve-plugin-path`, `declared-plugins`,
//! `loaded-plugins`.
//!
//! `%declare-plugin!` is the Rust backing for the Scheme-side `load-plugin`
//! wrapper defined in the bootstrap; see `mod.rs` for the Scheme source.

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::scripting::{SteelCtx, attribution::PluginId, hooks::HookId};

// ── Helpers ───────────────────────────────────────────────────────────────────

type SteelResult = Result<SteelVal, SteelErr>;

/// Convert a `PluginId::parse` error string into a Steel `Generic` error.
fn steel_parse_err(e: String) -> SteelErr {
    SteelErr::new(ErrorKind::Generic, e)
}

/// Extract a `Vec<String>` from a Steel list value.  Each element must be a
/// string; returns a typed error on any mismatch.
fn steel_list_to_strings(val: SteelVal, param: &'static str) -> Result<Vec<String>, SteelErr> {
    match val {
        SteelVal::ListV(list) => list
            .iter()
            .map(|v| match v {
                SteelVal::StringV(s) => Ok(s.to_string()),
                _ => steel::stop!(TypeMismatch => "{}: expected list of strings, got {:?}", param, v),
            })
            .collect(),
        _ => steel::stop!(TypeMismatch => "{}: expected a list, got {:?}", param, val),
    }
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(%declare-plugin! name on-command on-event on-language lazy)` — the single
/// Rust primitive backing the Scheme-side `load-plugin`.
///
/// - Validates `name`; aborts init on malformed names.
/// - Records into `declared_plugins` for PLUM compat.
/// - Parses trigger lists; converts event names to `HookId` variants.
/// - Registers the plugin in `LazyRegistry`.
/// - Eager plugins (no triggers, `lazy = #f`) are queued in
///   `eager_plugin_loads` for Phase-2 drain via `activate_plugin`.
pub(crate) fn declare_plugin(
    ctx: &mut SteelCtx,
    name: String,
    on_command: SteelVal,
    on_event: SteelVal,
    on_language: SteelVal,
    lazy: bool,
) -> SteelResult {
    let plugin_id = PluginId::parse(&name).map_err(steel_parse_err)?;

    // PLUM compat: declared_plugins always records every declared plugin.
    if !ctx
        .declared_plugins
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&name))
    {
        ctx.declared_plugins.push(name.clone());
    }

    let on_cmd = steel_list_to_strings(on_command, "on-command")?;
    let on_evt_strs = steel_list_to_strings(on_event, "on-event")?;
    let on_lang = steel_list_to_strings(on_language, "on-language")?;

    let on_evt: Vec<HookId> = on_evt_strs
        .iter()
        .map(|s| {
            HookId::from_symbol(s).ok_or_else(|| {
                let valid = HookId::all_names().collect::<Vec<_>>().join(", ");
                steel_parse_err(format!(
                    "on-event: unknown hook '{}'; valid: {}",
                    s, valid
                ))
            })
        })
        .collect::<Result<_, _>>()?;

    // Filter out any colliding trigger names before writing any state. Each
    // collision logs a non-fatal Error (visible in :messages) and the name is
    // dropped, so cmd_owners and command_triggers stay clean. A plugin whose
    // entire on-command list collides stays dead-lazy (had_command_triggers
    // keeps is_lazy true so it is never flipped to eager-load).
    let had_command_triggers = !on_cmd.is_empty();
    let mut valid = Vec::with_capacity(on_cmd.len());
    for cmd in on_cmd {
        if ctx.builtin_cmd_names.contains(&cmd) {
            ctx.log(
                crate::editor::Severity::Error,
                format!("load-plugin: command '{cmd}' conflicts with a built-in; trigger ignored"),
            );
        } else if ctx.lazy_registry.command_triggers.contains_key(&cmd) {
            ctx.log(
                crate::editor::Severity::Error,
                format!("load-plugin: command '{cmd}' already claimed by another lazy plugin; trigger ignored"),
            );
        } else {
            valid.push(cmd);
        }
    }
    let on_cmd = valid;

    let is_lazy = lazy || had_command_triggers || !on_evt.is_empty() || !on_lang.is_empty();

    let path = resolve_path_for_name(&name, ctx.runtime_dir, ctx.data_dir)
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;

    // Pre-seed cmd_owners so (command-plugin "cmd") resolves correctly before
    // the plugin body is evaluated (before activation).
    for cmd in &on_cmd {
        ctx.cmd_owners.insert(cmd.clone(), plugin_id.to_string());
    }

    ctx.lazy_registry
        .declare(plugin_id.clone(), path, on_cmd, on_evt, on_lang);

    if !is_lazy && ctx.lazy_registry.plugins.contains_key(&plugin_id) {
        ctx.eager_plugin_loads.push(plugin_id);
    }

    Ok(SteelVal::Void)
}

/// `(push-current-plugin! name)` — push `name` onto the `CURRENT_PLUGIN`
/// attribution stack.  Called from `dynamic-wind`'s before-thunk inside
/// the Scheme-side `load-plugin`.
pub(crate) fn push_current_plugin(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let plugin_id = PluginId::parse(&name).map_err(steel_parse_err)?;
    ctx.plugin_stack.push(plugin_id);
    Ok(SteelVal::Void)
}

/// `(pop-current-plugin!)` — pop the top entry from the `CURRENT_PLUGIN`
/// stack.  Called from `dynamic-wind`'s after-thunk.  Raises a Steel error
/// on empty stack (the before/after pairing should always be balanced).
pub(crate) fn pop_current_plugin(ctx: &mut SteelCtx) -> SteelResult {
    if ctx.plugin_stack.is_empty() {
        steel::stop!(Generic => "pop-current-plugin!: attribution stack is already empty");
    }
    ctx.plugin_stack.pop();
    Ok(SteelVal::Void)
}

/// Pure path resolution: given a plugin name and the runtime / data directories,
/// return the resolved `PathBuf` if the plugin file exists on disk, or `None`.
///
/// Called by the `resolve-plugin-path` Steel builtin (which accesses the dirs
/// via `&mut SteelCtx`).
pub(crate) fn resolve_path_for_name(
    name: &str,
    runtime_dir: Option<&std::path::Path>,
    data_dir: Option<&std::path::Path>,
) -> Result<Option<std::path::PathBuf>, String> {
    let plugin_id = PluginId::parse(name)?;
    let path = match plugin_id {
        PluginId::Core(core_name) => runtime_dir.map(|rt| {
            rt.join("plugins")
                .join("core")
                .join(&core_name)
                .join("plugin.scm")
        }),
        // When data_dir is None (HOME/APPDATA unset), user plugins cannot be
        // resolved — return None rather than panicking.
        PluginId::User { user, repo } => {
            data_dir.map(|d| d.join("plugins").join(&user).join(&repo).join("plugin.scm"))
        }
    };
    // Probe existence without a pre-flight `.exists()` to avoid TOCTOU.
    // NotFound → plugin absent (Ok(None)); other errors propagate.
    match path {
        None => Ok(None),
        Some(p) => match crate::os::fs::metadata(&p) {
            Ok(_) => Ok(Some(p)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot stat plugin path '{}': {e}", p.display())),
        },
    }
}

/// `(resolve-plugin-path name)` — return the resolved path string if the
/// plugin file exists on disk, or `#f` if absent.  Raises a Steel error for
/// malformed names.
pub(crate) fn resolve_plugin_path(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let path = resolve_path_for_name(&name, ctx.runtime_dir, ctx.data_dir)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    match path {
        Some(p) => Ok(SteelVal::StringV(p.to_string_lossy().into_owned().into())),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(loaded-plugins)` — return a Steel list of plugin names in `Loaded` state.
///
/// Derived from `LazyRegistry` so lazy plugins correctly read as not-yet-loaded
/// until their body has been evaluated.
pub(crate) fn loaded_plugins(ctx: &mut SteelCtx) -> SteelResult {
    use crate::scripting::lazy::PluginState;
    let vals: Vec<SteelVal> = ctx
        .lazy_registry
        .plugins
        .iter()
        .filter(|(_, state)| matches!(state, PluginState::Loaded))
        .map(|(id, _)| SteelVal::StringV(id.to_string().into()))
        .collect();
    vals.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

/// `(declared-plugins)` — return a Steel list of all declared third-party
/// (non-`core:*`) plugin names.  Used by PLUM to know what to install.
pub(crate) fn declared_plugins(ctx: &mut SteelCtx) -> SteelResult {
    let vals: Vec<SteelVal> = ctx
        .declared_plugins
        .iter()
        .filter(|name| !name.to_ascii_lowercase().starts_with("core:"))
        .map(|s| SteelVal::StringV(s.as_str().into()))
        .collect();
    vals.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// Parsing tests (valid/invalid plugin names, segments) live in
// `scripting::attribution::tests` alongside `PluginId::parse`.  The tests here
// cover only the builtins' Steel-facing behaviour.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_core_plugin_name() {
        let id = PluginId::parse("core:helix-surround").unwrap();
        assert!(matches!(id, PluginId::Core(n) if n == "helix-surround"));
    }

    #[test]
    fn parse_user_plugin_name() {
        let id = PluginId::parse("user/repo").unwrap();
        assert!(
            matches!(id, PluginId::User { ref user, ref repo } if user == "user" && repo == "repo")
        );
    }

    #[test]
    fn parse_invalid_names() {
        assert!(PluginId::parse("bad").is_err());
        assert!(PluginId::parse("core:").is_err());
        assert!(PluginId::parse("a/b/c").is_err());
        assert!(PluginId::parse("/repo").is_err());
        assert!(PluginId::parse("user/").is_err());
        assert!(PluginId::parse("core:..").is_err());
        assert!(PluginId::parse("../evil").is_err());
    }

    #[test]
    fn valid_core_segment() {
        // Segments that should pass through PluginId::parse successfully.
        assert!(PluginId::parse("core:helix-surround").is_ok());
        assert!(PluginId::parse("core:plum").is_ok());
        assert!(PluginId::parse("core:v1.2.3").is_ok());
    }

    #[test]
    fn invalid_segments() {
        // Segment validation exercised via PluginId::parse.
        assert!(PluginId::parse("core:").is_err()); // empty
        assert!(PluginId::parse("core:.").is_err()); // dot
        assert!(PluginId::parse("core:..").is_err()); // dotdot
        assert!(PluginId::parse("./b").is_err()); // slash without user
        assert!(PluginId::parse("a\0b/repo").is_err()); // NUL in user
    }
}
