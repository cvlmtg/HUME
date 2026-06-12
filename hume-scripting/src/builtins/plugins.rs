//! Plugin lifecycle builtins: `%declare-plugin!`, `%load-plugin!`,
//! `resolve-plugin-path`, `declared-plugins`, `loaded-plugins`.
//!
//! `%declare-plugin!` backs the Scheme `declare-plugin` wrapper (lazy).
//! `%load-plugin!` backs the Scheme `load-plugin` wrapper (eager).
//! Both wrappers are defined in the bootstrap; see `builtins/mod.rs`.

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::{SteelCtx, attribution::PluginId, hooks::HookId, lazy::PluginState};

use super::list_to_strings;

// ── Helpers ───────────────────────────────────────────────────────────────────

type SteelResult = Result<SteelVal, SteelErr>;

/// Convert a `PluginId::parse` error string into a Steel `Generic` error.
fn steel_parse_err(e: String) -> SteelErr {
    SteelErr::new(ErrorKind::Generic, e)
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(%declare-plugin! name on-command on-event on-language)` — Rust primitive
/// backing the Scheme-side `declare-plugin` wrapper.
///
/// Always lazy: the plugin body is never evaluated here.  `activate_plugin`
/// runs it when a trigger fires or `(load-plugin name)` is called explicitly.
///
/// A bare `(declare-plugin name)` with no triggers is *not* redundant with
/// `(load-plugin name)`: both record the name for PLUM, but `declare-plugin`
/// defers body evaluation.  The key case is an on-demand dependency whose only
/// `(load-plugin "dep")` call sits inside another plugin's body — that in-body
/// call records "dep" only when the parent activates, which is too late for a
/// fresh machine where the absent dep causes a hard error before the parent can
/// ever run.  A top-level bare `(declare-plugin "dep")` records it up front so
/// PLUM installs it; after that the parent's in-body `(load-plugin "dep")`
/// succeeds.
///
/// - Validates `name`; aborts init on malformed names.
/// - Records into `declared_plugins` for PLUM compat.
/// - Parses trigger lists; converts event names to `HookId` variants.
/// - Filters colliding trigger names (logs `Severity::Error`, continues).
/// - Registers the plugin in `LazyRegistry`.
pub(crate) fn declare_plugin(
    ctx: &mut SteelCtx,
    name: String,
    on_command: SteelVal,
    on_event: SteelVal,
    on_language: SteelVal,
) -> SteelResult {
    let plugin_id = PluginId::parse(&name).map_err(steel_parse_err)?;

    // PLUM compat: declared_plugins always records every declared plugin.
    if !ctx
        .registries
        .declared_plugins
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&name))
    {
        ctx.registries.declared_plugins.push(name.clone());
    }

    let on_cmd = list_to_strings(on_command, "on-command")?;
    let on_evt_strs = list_to_strings(on_event, "on-event")?;
    let on_lang = list_to_strings(on_language, "on-language")?;

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

    // Filter colliding trigger names before writing any state.  Each collision
    // logs a non-fatal Error (visible in :messages) and the name is dropped, so
    // cmd_owners and command_triggers stay consistent.
    let mut valid = Vec::with_capacity(on_cmd.len());
    for cmd in on_cmd {
        if ctx.builtin_cmd_names.contains(&cmd) {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: command '{cmd}' conflicts with a built-in; trigger ignored"),
            );
        } else if ctx.registries.lazy_registry.command_triggers.contains_key(&cmd) {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: command '{cmd}' already claimed by another lazy plugin; trigger ignored"),
            );
        } else {
            valid.push(cmd);
        }
    }
    let on_cmd = valid;

    let path = resolve_path_for_name(&name, ctx.runtime_dir, ctx.data_dir)
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;

    // Pre-seed cmd_owners so (command-plugin "cmd") resolves correctly before
    // the plugin body is evaluated (before activation).
    for cmd in &on_cmd {
        ctx.registries.cmd_owners.insert(cmd.clone(), plugin_id.to_string());
    }

    ctx.registries.lazy_registry
        .declare(plugin_id, path, on_cmd, on_evt, on_lang);

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
        Some(p) => match hume_platform::fs::metadata(&p) {
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

/// `(%load-plugin! "name")` — Rust primitive backing the Scheme-side
/// `load-plugin` wrapper (eager).
///
/// Load-time only: valid from `init.scm` and plugin bodies (`is_init = true`).
///
/// If the plugin is not yet declared, resolves its path and registers it now:
/// - Top-level (`plugin_stack` empty): absent on disk → silent skip + record
///   in `declared_plugins` for PLUM to install on the next `:plum-install`.
/// - Inside a plugin body (`plugin_stack` non-empty): absent on disk → hard
///   error, because a missing dependency cannot be silently ignored.
///
/// If already declared (lazy or otherwise), queues it for activation.
/// If already `Loaded` or `Failed`, the `activate_plugin` idempotency guard
/// handles it as a no-op.
pub(crate) fn load_plugin(ctx: &mut SteelCtx, name: String) -> SteelResult {
    if !ctx.is_init {
        steel::stop!(Generic => "load-plugin: can only be called during init/plugin load");
    }
    let id = PluginId::parse(&name).map_err(steel_parse_err)?;

    // PLUM compat: record name regardless of disk presence.
    if !ctx
        .registries
        .declared_plugins
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&name))
    {
        ctx.registries.declared_plugins.push(name.clone());
    }

    if !ctx.registries.lazy_registry.plugins.contains_key(&id) {
        let path = resolve_path_for_name(&name, ctx.runtime_dir, ctx.data_dir)
            .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;
        match path {
            Some(p) => {
                ctx.registries.lazy_registry
                    .plugins
                    .insert(id.clone(), PluginState::Declared { path: p });
            }
            None => {
                if ctx.plugin_stack.is_empty() {
                    // Top-level: absent is PLUM-friendly; already recorded for install.
                    return Ok(SteelVal::Void);
                }
                steel::stop!(Generic =>
                    "load-plugin: dependency '{}' not found on disk", name);
            }
        }
    }

    Ok(SteelVal::Void)
}

/// Maximum nesting depth for concurrent inline plugin activations.
///
/// A depth of 16 is unreachable in practice (plugins rarely chain more than
/// 2–3 levels deep) but stops a misconfigured cycle that slipped past the
/// `Loading` guard from recursing until the stack overflows.
const MAX_ACTIVATION_DEPTH: usize = 16;

/// `(%begin-lazy-activation id-str)` — Rust primitive for inline activation.
///
/// Called from the BOOTSTRAP `%activate-plugin-inline` helper immediately before
/// `(hm.eval-string require-string)`.  If the plugin is `Declared`, transitions
/// to `Loading`, pushes `plugin_stack`, increments `activation_depth`, and
/// returns the `(require "<abs>")` string.  Returns `#f` for the cycle/idempotency
/// guard (Loading/Loaded/Failed/absent) so `%activate-plugin-inline` becomes a
/// no-op without error.
pub(crate) fn begin_lazy_activation(ctx: &mut SteelCtx, id_str: String) -> SteelResult {
    let id = PluginId::parse(&id_str).map_err(steel_parse_err)?;

    let path = match ctx.registries.lazy_registry.plugins.get(&id) {
        Some(PluginState::Declared { path }) => path.clone(),
        Some(PluginState::Loading | PluginState::Loaded | PluginState::Failed) | None => {
            return Ok(SteelVal::BoolV(false));
        }
    };

    if ctx.registries.activation_depth >= MAX_ACTIVATION_DEPTH {
        ctx.registries.lazy_registry.plugins.insert(id.clone(), PluginState::Failed);
        steel::stop!(Generic =>
            "%begin-lazy-activation: activation depth limit ({}) exceeded — \
             check for circular load-plugin chains; '{}' marked Failed",
            MAX_ACTIVATION_DEPTH, id_str);
    }

    let abs_str = path.to_string_lossy();
    if abs_str.contains('"') {
        ctx.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Failed);
        steel::stop!(Generic =>
            "plugin path contains '\"' — cannot embed in require: {}", path.display());
    }
    let require_program = format!("(require \"{abs_str}\")");

    ctx.registries.lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Loading);
    ctx.plugin_stack.push(id);
    ctx.registries.activation_depth += 1;

    Ok(SteelVal::StringV(require_program.into()))
}

/// `(%finish-lazy-activation id-str success?)` — Rust primitive for inline activation.
///
/// Called from `%activate-plugin-inline` after `(hm.eval-string …)` completes
/// (or fails).  Pops `plugin_stack`, decrements `activation_depth`, and
/// transitions the plugin to `Loaded` (success) or `Failed` (failure).
/// `drop_triggers_for` runs on both paths so expired trigger entries are cleaned up.
///
/// **Partial-failure note**: Steel's `eval-string` (used by
/// `%activate-plugin-inline`) has no transactional semantics.  Any `define`
/// that completed before the error stays in the VM's symbol table even when we
/// transition to `Failed`.  These orphaned globals are harmless but visible;
/// `command_table` is the authoritative dispatch table for HUME commands.
pub(crate) fn finish_lazy_activation(
    ctx: &mut SteelCtx,
    id_str: String,
    success: bool,
) -> SteelResult {
    let id = PluginId::parse(&id_str).map_err(steel_parse_err)?;

    ctx.plugin_stack.pop();
    ctx.registries.activation_depth = ctx.registries.activation_depth.saturating_sub(1);

    let new_state = if success { PluginState::Loaded } else { PluginState::Failed };
    ctx.registries.lazy_registry.plugins.insert(id.clone(), new_state);
    ctx.registries.lazy_registry.drop_triggers_for(&id);

    Ok(SteelVal::Void)
}

/// `(%lazy-command-owner name)` — return the owning plugin's id string if `name`
/// is a registered command trigger, or `#f` if not.  Used by `%dispatch-command`
/// to decide whether a `command_table` miss should trigger inline activation.
pub(crate) fn lazy_command_owner(ctx: &mut SteelCtx, name: String) -> SteelResult {
    match ctx.registries.lazy_registry.command_triggers.get(&name) {
        Some(id) => Ok(SteelVal::StringV(id.to_string().into())),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(loaded-plugins)` — return a Steel list of plugin names in `Loaded` state.
///
/// Derived from `LazyRegistry` so lazy plugins correctly read as not-yet-loaded
/// until their body has been evaluated.
pub(crate) fn loaded_plugins(ctx: &mut SteelCtx) -> SteelResult {
    let vals: Vec<SteelVal> = ctx
        .registries
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
        .registries
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
// `hume_scripting::attribution::tests` alongside `PluginId::parse`.  The tests here
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
