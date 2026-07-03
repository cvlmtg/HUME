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

/// Log an `Error` for a `core:` plugin that is absent from the runtime dir.
///
/// For `core:` plugins, absent = typo or broken `HUME_RUNTIME`; PLUM never
/// installs them (they are bundled), so there is no "install and reload" path.
fn log_absent_core(ctx: &mut SteelCtx, name: &str, verb: &str) {
    ctx.log(
        crate::log::LogLevel::Error,
        format!(
            "{verb}: unknown core plugin '{name}' — not found in runtime dir \
             (typo, or HUME_RUNTIME misconfigured)"
        ),
    );
}

/// Gate for plugin-registration verbs (`load-plugin`, `declare-plugin`).
///
/// Both verbs are valid only at the top level of `init.scm`.  A plugin can
/// never load or declare another plugin — dependency declarations are the
/// user's / plugin-manager's responsibility, not a plugin's.
///
/// At init.scm top level: `is_init = true` and `plugin_stack` is empty.
/// Inside any eager plugin body: `is_init = true` but `plugin_stack` is non-empty.
/// Inside any lazy/runtime plugin body: `is_init = false`.
fn ensure_top_level(ctx: &SteelCtx, verb: &str) -> Result<(), SteelErr> {
    if !ctx.is_init || !ctx.plugin_stack.is_empty() {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            format!(
                "{verb}: can only be called at the top level of init.scm, \
                 not from a plugin body"
            ),
        ));
    }
    Ok(())
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(%declare-plugin! name commands events languages)` — Rust primitive
/// backing the Scheme-side `declare-plugin` wrapper.
///
/// Top-level only: valid only at the top level of `init.scm`.  A plugin can
/// never declare another plugin — see `ensure_top_level`.
///
/// `declare-plugin` is the plugin **manifest**: it records what the plugin
/// offers the editor (commands it provides, languages/events it reacts to).
/// Unlike `load-plugin` (eager: body evaluated immediately), `declare-plugin`
/// defers body evaluation until the first activation entry is exercised.  Both
/// are registration verbs that record the plugin for PLUM; the verb choice
/// encodes eager vs. lazy body evaluation.  At least one activation entry is
/// required — a manifest with no entries hard-errors because the plugin could
/// never be activated.
///
/// - Validates `name`; aborts init on malformed names.
/// - Records into `declared_plugins` for PLUM compat.
/// - Parses activation entry lists; converts event names to `HookId` variants.
/// - Filters colliding command entries (logs `Severity::Error`, continues).
/// - Registers the plugin in `LazyRegistry`.
pub(crate) fn declare_plugin(
    ctx: &mut SteelCtx,
    name: String,
    commands: SteelVal,
    events: SteelVal,
    languages: SteelVal,
) -> SteelResult {
    ensure_top_level(ctx, "declare-plugin")?;
    let plugin_id = PluginId::parse(&name).map_err(steel_parse_err)?;

    // If the plugin is already in the registry, decide by state:
    // - Loaded: soft error (prior load-plugin contradicts this declare).
    // - Declared/Loading/Failed: silent no-op (first declaration wins; idempotency).
    match ctx.registries.lazy_registry.plugins.get(&plugin_id) {
        Some(PluginState::Loaded) => {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: '{name}' is already loaded; ignoring declare"),
            );
            return Ok(SteelVal::Void);
        }
        Some(_) => return Ok(SteelVal::Void), // Declared/Loading/Failed: first wins
        None => {}
    }

    // PLUM compat: declared_plugins always records every declared plugin.
    if !ctx
        .registries
        .declared_plugins
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&name))
    {
        ctx.registries.declared_plugins.push(name.clone());
    }

    let cmd_list = list_to_strings(commands, "commands")?;
    let evt_strs = list_to_strings(events, "events")?;
    let lang_list = list_to_strings(languages, "languages")?;

    // Captured before the collision-filter loop moves `cmd_list`.  Used below to
    // distinguish "all commands collided" from "none were supplied" in the
    // zero-entry error message.
    let had_commands = !cmd_list.is_empty();

    let evt_list: Vec<HookId> = evt_strs
        .iter()
        .map(|s| {
            HookId::from_symbol(s).ok_or_else(|| {
                let valid = HookId::all_names().collect::<Vec<_>>().join(", ");
                steel_parse_err(format!("events: unknown hook '{}'; valid: {}", s, valid))
            })
        })
        .collect::<Result<_, _>>()?;

    // Filter colliding command names before writing any state.  Each collision
    // logs a non-fatal Error (visible in :messages) and the name is dropped, so
    // cmd_owners and activation_commands stay consistent.
    let mut valid = Vec::with_capacity(cmd_list.len());
    for cmd in cmd_list {
        // Malformed name (not a collision) → hard error, same rule as
        // define-command!.  A name that can't survive quoting is a typo.
        if cmd.contains('"') || cmd.contains('\\') {
            steel::stop!(Generic =>
                "declare-plugin: command name '{}' must not contain '\"' or '\\'", cmd);
        }
        if ctx.builtin_cmd_names.contains(&cmd) {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: command '{cmd}' conflicts with a built-in; activation entry ignored"),
            );
        } else if ctx
            .registries
            .lazy_registry
            .activation_commands
            .contains_key(&cmd)
        {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: command '{cmd}' already claimed by another lazy plugin; activation entry ignored"),
            );
        } else if ctx.registries.command_table.contains_key(&cmd) {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: command '{cmd}' is already defined by an eager command; activation entry ignored"),
            );
        } else {
            valid.push(cmd);
        }
    }
    let cmd_list = valid;

    // Hard error: no usable activation entries after collision filtering means
    // the plugin can never be activated at runtime.  Use load-plugin instead.
    if cmd_list.is_empty() && evt_list.is_empty() && lang_list.is_empty() {
        let msg = if had_commands {
            // User supplied #:commands entries but all were filtered by collision
            // checks.  Telling them to "Add #:commands" would be misleading.
            format!(
                "declare-plugin: '{name}' declares no activation entries; \
                 all #:commands entries conflicted with existing commands. \
                 Fix the collision or use (load-plugin \"{name}\") for eager loading."
            )
        } else {
            format!(
                "declare-plugin: '{name}' declares no activation entries; it could never be activated. \
                 Add #:commands/#:events/#:languages, or use (load-plugin \"{name}\") for eager loading."
            )
        };
        return Err(steel_parse_err(msg));
    }

    let path = resolve_path_for_name(&name, ctx.runtime_dir, ctx.data_dir)
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;

    // When the plugin file is absent on disk, LazyRegistry::declare would silently
    // skip it (no activation entries, no state).  For user/ plugins, log Info — absent is
    // expected before :plum-install.  For core: plugins, absent means a typo or
    // broken HUME_RUNTIME; PLUM never installs core: plugins, so it can't catch
    // the error.  `declared_plugins` is already recorded above for PLUM.
    if path.is_none() {
        match &plugin_id {
            PluginId::Core(_) => log_absent_core(ctx, &name, "declare-plugin"),
            PluginId::User { .. } => ctx.log(
                crate::log::LogLevel::Info,
                format!(
                    "declare-plugin: '{name}' not found on disk; install and reload to activate."
                ),
            ),
        }
        return Ok(SteelVal::Void);
    }

    // Pre-seed cmd_owners so (command-plugin "cmd") resolves correctly before
    // the plugin body is evaluated (before activation).  Only reached when the
    // plugin exists on disk: if it is absent, the early return above fires before
    // this point and LazyRegistry::declare is never called — seeding here for a
    // missing plugin would create orphan attribution entries that drop_activations_for
    // can never clean up (it only fires on load/fail, not on absent-path skips).
    for cmd in &cmd_list {
        ctx.registries
            .cmd_owners
            .insert(cmd.clone(), plugin_id.to_string());
    }

    ctx.registries
        .lazy_registry
        .declare(plugin_id, path, cmd_list, evt_list, lang_list);

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
/// Top-level only: valid only at the top level of `init.scm`.  A plugin can
/// never load another plugin — see `ensure_top_level`.
///
/// If the plugin is not yet declared, resolves its path and registers it now:
/// absent on disk → silent skip + record in `declared_plugins` for PLUM to
/// install on the next `:plum-install`.
///
/// If already declared (lazy or otherwise), queues it for activation.
/// If already `Loaded` or `Failed`, the `activate_plugin` idempotency guard
/// handles it as a no-op.
pub(crate) fn load_plugin(ctx: &mut SteelCtx, name: String) -> SteelResult {
    ensure_top_level(ctx, "load-plugin")?;
    let id = PluginId::parse(&name).map_err(steel_parse_err)?;

    // Soft error: if this plugin was already declared lazily, loading it eagerly
    // contradicts the declare.  Warn and fall through — the wrapper still activates it.
    if matches!(
        ctx.registries.lazy_registry.plugins.get(&id),
        Some(PluginState::Declared { .. })
    ) {
        ctx.log(
            crate::log::LogLevel::Error,
            format!(
                "load-plugin: '{name}' was already declared lazily; \
                 load-plugin overrides and forces eager loading"
            ),
        );
    }

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
                ctx.registries
                    .lazy_registry
                    .plugins
                    .insert(id.clone(), PluginState::Declared { path: p });
            }
            None => {
                // core: absent → error (typo or HUME_RUNTIME broken; PLUM won't catch it).
                // user/ absent → silent (PLUM installs it on :plum-install).
                if matches!(&id, PluginId::Core(_)) {
                    log_absent_core(ctx, &name, "load-plugin");
                }
                return Ok(SteelVal::Void);
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
/// to `Loading`, pushes `plugin_stack`, and returns the `(require "<abs>")` string.
/// Returns `#f` for the cycle/idempotency guard (Loading/Loaded/Failed/absent) so
/// `%activate-plugin-inline` becomes a no-op without error.
pub(crate) fn begin_lazy_activation(ctx: &mut SteelCtx, id_str: String) -> SteelResult {
    let id = PluginId::parse(&id_str).map_err(steel_parse_err)?;

    let path = match ctx.registries.lazy_registry.plugins.get(&id) {
        Some(PluginState::Declared { path }) => path.clone(),
        Some(PluginState::Loading | PluginState::Loaded | PluginState::Failed) | None => {
            return Ok(SteelVal::BoolV(false));
        }
    };

    if ctx.plugin_stack.len() >= MAX_ACTIVATION_DEPTH {
        ctx.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Failed);
        steel::stop!(Generic =>
            "%begin-lazy-activation: activation depth limit ({}) exceeded — \
             check for circular load-plugin chains; '{}' marked Failed",
            MAX_ACTIVATION_DEPTH, id_str);
    }

    let abs_str = path.to_string_lossy();
    if abs_str.contains('"') {
        ctx.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Failed);
        steel::stop!(Generic =>
            "plugin path contains '\"' — cannot embed in require: {}", path.display());
    }
    // Escape backslashes so Windows paths (e.g. `C:\Users\…`) survive
    // embedding inside a Steel string literal — `\U` etc. are invalid escapes.
    let escaped = abs_str.replace('\\', "\\\\");
    let require_program = format!("(require \"{escaped}\")");

    ctx.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Loading);
    ctx.plugin_stack.push(id);

    Ok(SteelVal::StringV(require_program.into()))
}

/// `(%finish-lazy-activation id-str success?)` — Rust primitive for inline activation.
///
/// Called from `%activate-plugin-inline` after `(hm.eval-string …)` completes
/// (or fails).  Pops `plugin_stack` and transitions the plugin to `Loaded`
/// (success) or `Failed` (failure).
/// `drop_activations_for` runs on both paths so expired activation entries are cleaned up.
///
/// On failure, any commands that a partially-evaluated body already registered via
/// `define-command!` are rolled back: removed from `command_table`, `cmd_owners`,
/// and the editor's `CommandRegistry`.  This prevents a `Failed` plugin from
/// leaving callable orphan commands behind.  Steel globals defined before the
/// error remain in the VM's symbol table (no rollback possible there) but are
/// unreachable through HUME's command dispatch.
pub(crate) fn finish_lazy_activation(
    ctx: &mut SteelCtx,
    id_str: String,
    success: bool,
) -> SteelResult {
    let id = PluginId::parse(&id_str).map_err(steel_parse_err)?;

    ctx.plugin_stack.pop();

    let new_state = if success {
        PluginState::Loaded
    } else {
        PluginState::Failed
    };
    ctx.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), new_state);
    ctx.registries.lazy_registry.drop_activations_for(&id);

    if !success {
        // Roll back any commands the failed body partially registered.
        let id_str_owned = id.to_string();
        let orphans: Vec<String> = ctx
            .registries
            .cmd_owners
            .iter()
            .filter(|(_, owner)| *owner == &id_str_owned)
            .map(|(name, _)| name.clone())
            .collect();
        for name in orphans {
            ctx.registries.command_table.remove(&name);
            ctx.registries.cmd_owners.remove(&name);
            ctx.host.unregister_command(&name);
        }
    }

    Ok(SteelVal::Void)
}

/// `(%lazy-command-owner name)` — return the owning plugin's id string if `name`
/// is a registered activation command, or `#f` if not.  Used by `%dispatch-command`
/// to decide whether a `command_table` miss should trigger inline activation.
pub(crate) fn lazy_command_owner(ctx: &mut SteelCtx, name: String) -> SteelResult {
    match ctx.registries.lazy_registry.activation_commands.get(&name) {
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

    // ── Activation depth cap ──────────────────────────────────────────────────

    /// `%begin-lazy-activation` refuses to start when `plugin_stack` depth is at
    /// `MAX_ACTIVATION_DEPTH`, marks the plugin `Failed`, and returns a Steel error.
    ///
    /// Fail oracle: remove the depth-cap check from `begin_lazy_activation` →
    /// an infinite cycle would stack-overflow instead of hard-erroring.
    #[test]
    fn begin_lazy_activation_at_depth_cap_errors_and_marks_failed() {
        use crate::{ScriptingHost, null_host::NullHost};
        use std::io::Write as _;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("deep.scm");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"(define x 1)")
            .unwrap();

        let id = PluginId::parse("core:deep").unwrap();
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });
        // Simulate maximum nesting depth already reached by seeding the stack.
        let dummy = PluginId::parse("core:dummy").unwrap();
        for _ in 0..MAX_ACTIVATION_DEPTH {
            host.push_plugin_for_test(dummy.clone());
        }

        let result = host.eval_source(r#"(%begin-lazy-activation "core:deep")"#, &mut NullHost);

        assert!(
            result.is_err(),
            "depth cap must raise a Steel error; got Ok"
        );
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Failed)
            ),
            "plugin must be marked Failed when depth cap exceeded"
        );
    }

    /// `%begin-lazy-activation` at depth cap − 1 succeeds (cap is exclusive).
    ///
    /// Confirms the off-by-one is correct: depth 15 of 16 is still allowed.
    #[test]
    fn begin_lazy_activation_below_depth_cap_succeeds() {
        use crate::{ScriptingHost, null_host::NullHost};
        use std::io::Write as _;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ok.scm");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"(define x 1)")
            .unwrap();

        let id = PluginId::parse("core:ok").unwrap();
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });
        // One below the cap — must still be allowed.
        let dummy = PluginId::parse("core:dummy").unwrap();
        for _ in 0..MAX_ACTIVATION_DEPTH - 1 {
            host.push_plugin_for_test(dummy.clone());
        }

        // Transition to Loading and return the require-string (not an error).
        let result = host.eval_source(r#"(%begin-lazy-activation "core:ok")"#, &mut NullHost);
        assert!(result.is_ok(), "depth below cap must be allowed; got Err");
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loading)
            ),
            "plugin must be Loading after successful %begin-lazy-activation"
        );
    }

    // ── Command-name character validation ─────────────────────────────────────

    /// `declare-plugin` hard-errors on a `#:commands` entry containing `"` or
    /// `\` — the same rule `define-command!` enforces.
    ///
    /// Fail oracle: remove the character check from the filter loop → the name
    /// registers as an activation entry and the declare succeeds.
    #[test]
    fn declare_plugin_command_name_with_quote_errors() {
        use crate::{ScriptingHost, null_host::NullHost};
        let mut host = ScriptingHost::new();
        let result = host.eval_source(
            r#"(declare-plugin "user/tp" #:commands '("bad\"name"))"#,
            &mut NullHost,
        );
        let err = result.expect_err("quoted command name must be rejected");
        assert!(
            err.contains("must not contain"),
            "error must name the invalid character rule; got: {err}"
        );
        assert!(
            host.registries.lazy_registry.activation_commands.is_empty(),
            "no activation entry may be recorded for a malformed name"
        );
    }

    // ── G1: cmd_owners not seeded for absent-path plugins ─────────────────────

    /// When a declared plugin is absent on disk, `cmd_owners` must NOT be pre-seeded.
    ///
    /// The old bug: `cmd_owners` was seeded before the path check, so an absent
    /// plugin left orphan attribution entries that could never be cleaned up by
    /// `drop_activations_for`.  The fix: the absent-path early-return fires before
    /// the pre-seed loop.
    ///
    /// Fail oracle: remove the `if path.is_none() { return Ok(…) }` early-return →
    /// cmd_owners gets seeded → assertion fires.
    #[test]
    fn declare_plugin_absent_on_disk_does_not_seed_cmd_owners() {
        use crate::{ScriptingHost, null_host::NullHost};
        let mut host = ScriptingHost::new();
        // `core:nonexistent-plugin` cannot exist on disk in any test environment.
        let result = host.eval_source(
            r#"(declare-plugin "core:nonexistent-plugin" #:commands '("my-cmd"))"#,
            &mut NullHost,
        );
        assert!(
            result.is_ok(),
            "absent-path declare-plugin must not error; got: {result:?}"
        );
        assert!(
            !host.cmd_owners_for_test().contains_key("my-cmd"),
            "cmd_owners must not be seeded when the plugin is absent on disk"
        );
    }

    // ── G3: zero-entry error distinguishes collided vs not-supplied ───────────

    /// When ALL provided `#:commands` entries collide with built-ins, the
    /// error message must mention "conflicted", not suggest adding #:commands
    /// (which the user already did).
    ///
    /// Fail oracle: remove the `had_commands` branch → generic "Add #:commands"
    /// message → second assertion fires.
    #[test]
    fn declare_plugin_all_on_command_collided_message_mentions_conflict() {
        use crate::{ScriptingHost, null_host::NullHost};
        use std::collections::HashSet;
        let mut host = ScriptingHost::new();
        // Mark "insert-mode" as a built-in so collision filtering drops it.
        let mut builtin_names = HashSet::new();
        builtin_names.insert("insert-mode".to_string());

        let result = host.eval_source_returning_defs(
            r#"(declare-plugin "core:test-collision" #:commands '("insert-mode"))"#.to_owned(),
            builtin_names,
            &mut NullHost,
        );

        let err = result.expect_err("must error when all entries collide");
        assert!(
            err.contains("conflicted"),
            "error must mention the collision; got: {err}"
        );
        assert!(
            !err.contains("Add #:commands"),
            "must not suggest adding what user already provided; got: {err}"
        );
    }

    // ── G4: absent plugin logging ──────────────────────────────────────────────

    /// `declare-plugin "core:X"` absent on disk → `Error` log (typo / broken
    /// HUME_RUNTIME; PLUM never installs core: plugins so it can't catch this).
    ///
    /// Fail oracle: remove `log_absent_core` call → no Error message → assertion fires.
    #[test]
    fn declare_plugin_core_absent_logs_error() {
        use crate::{ScriptingHost, null_host::NullHost};
        let mut host = ScriptingHost::new();
        let result = host.eval_source(
            r#"(declare-plugin "core:nonexistent-plugin" #:commands '("my-cmd"))"#,
            &mut NullHost,
        );
        assert!(
            result.is_ok(),
            "absent core: declare must be non-fatal; got: {result:?}"
        );
        let messages = host.peek_pending_messages();
        assert!(
            messages.iter().any(|(level, msg)| {
                matches!(level, crate::log::LogLevel::Error)
                    && msg.contains("unknown core plugin")
                    && msg.contains("core:nonexistent-plugin")
            }),
            "must log Error for absent core: plugin; messages: {messages:?}"
        );
        assert!(
            !messages
                .iter()
                .any(|(_, msg)| msg.contains("install and reload")),
            "must not suggest install for core: plugin; messages: {messages:?}"
        );
    }

    /// `declare-plugin "user/X"` absent on disk → `Info` log (not yet installed;
    /// PLUM will surface it on :plum-install — no change needed in HUME).
    ///
    /// Fail oracle: swap Info→Error → assertion fires.
    #[test]
    fn declare_plugin_user_absent_logs_info() {
        use crate::{ScriptingHost, null_host::NullHost};
        let mut host = ScriptingHost::new();
        let result = host.eval_source(
            r#"(declare-plugin "user/definitely-absent-99" #:commands '("my-cmd-2"))"#,
            &mut NullHost,
        );
        assert!(
            result.is_ok(),
            "absent user/ declare must be non-fatal; got: {result:?}"
        );
        let messages = host.peek_pending_messages();
        assert!(
            messages.iter().any(|(level, msg)| {
                matches!(level, crate::log::LogLevel::Info) && msg.contains("not found on disk")
            }),
            "must log Info for absent user/ plugin; messages: {messages:?}"
        );
    }

    /// `load-plugin "core:X"` absent on disk → `Error` log (was silently swallowed).
    ///
    /// Fail oracle: remove `log_absent_core` call in load_plugin → no Error message → assertion fires.
    #[test]
    fn load_plugin_core_absent_logs_error() {
        use crate::{ScriptingHost, null_host::NullHost};
        let mut host = ScriptingHost::new();
        let result = host.eval_source(r#"(load-plugin "core:nonexistent-plugin")"#, &mut NullHost);
        assert!(
            result.is_ok(),
            "absent core: load must be non-fatal; got: {result:?}"
        );
        let messages = host.peek_pending_messages();
        assert!(
            messages.iter().any(|(level, msg)| {
                matches!(level, crate::log::LogLevel::Error)
                    && msg.contains("unknown core plugin")
                    && msg.contains("core:nonexistent-plugin")
            }),
            "must log Error for absent core: load-plugin; messages: {messages:?}"
        );
    }

    // ── G4: activation command name collision (symmetric checks) ─────────────

    /// `define-command!` rejects a name already in `activation_commands` (claimed
    /// by a lazy plugin), even when the eager `define-command!` runs first.
    ///
    /// Fail oracle: remove the `activation_commands` guard from `define_command_inner`
    /// → the eager define succeeds, the activation entry is orphaned, the plugin
    /// is stuck `Declared` and can never load.
    #[test]
    fn define_command_rejects_name_claimed_by_lazy_plugin() {
        use crate::{ScriptingHost, null_host::NullHost};

        let id = PluginId::parse("core:my-plugin").unwrap();
        let mut host = ScriptingHost::new();
        // Simulate declare-plugin having claimed the name as an activation entry.
        host.registries
            .lazy_registry
            .activation_commands
            .insert("my-lazy-cmd".to_string(), id);

        let result = host.eval_source(
            r#"(define-command! "my-lazy-cmd" "doc" (lambda () 0))"#,
            &mut NullHost,
        );

        assert!(
            result.is_err(),
            "define-command! must reject a name claimed by a lazy plugin; got Ok"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("claimed as an activation command"),
            "error must name the collision; got: {err}"
        );
        // The activation entry must survive — only drop_activations_for removes it (on load/fail).
        assert!(
            host.registries
                .lazy_registry
                .activation_commands
                .contains_key("my-lazy-cmd"),
            "activation_commands entry must not be removed by the failed define-command!"
        );
    }

    /// `declare-plugin` drops `#:commands` entries that conflict with already-registered
    /// eager commands; when the dropped entry was the sole activation signal, it errors
    /// immediately (no orphan entry, no plugin stuck `Declared`).
    ///
    /// Fail oracle: remove the `command_table` check from the `declare_plugin` filter
    /// loop → the name slips into `activation_commands` as an orphan and the
    /// "no activation entries" error is not raised.
    #[test]
    #[cfg(not(windows))]
    fn declare_plugin_drops_sole_command_conflicting_with_eager() {
        use crate::{ScriptingHost, null_host::NullHost};
        use steel::rvals::SteelVal;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        // Plugin file must exist so declare-plugin proceeds past the path check.
        let plugin_dir = dir.path().join("plugins").join("user").join("test-repo");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();

        let mut host = ScriptingHost::new();
        host.set_data_dir(dir.path().to_path_buf());
        // Simulate an eager command already occupying the name.
        host.registries
            .command_table
            .insert("my-eager-cmd".to_string(), SteelVal::Void);

        let result = host.eval_source(
            r#"(declare-plugin "user/test-repo" #:commands '("my-eager-cmd"))"#,
            &mut NullHost,
        );

        // All entries filtered → declare-plugin must fail with "no activation entries".
        let err = result.expect_err(
            "declare-plugin must error when sole #:commands entry is taken by an eager command",
        );
        assert!(
            err.contains("no activation entries") || err.contains("conflicted"),
            "error must explain the cause; got: {err}"
        );
        // Must not pollute activation_commands with the orphan entry.
        assert!(
            !host
                .registries
                .lazy_registry
                .activation_commands
                .contains_key("my-eager-cmd"),
            "activation_commands must not be polluted with the conflicting eager command name"
        );
    }

    // ── ScriptingHost::drop_activation_command ────────────────────────────────

    /// `drop_activation_command` removes the named command from both
    /// `activation_commands` and `cmd_owners`, leaving unrelated entries intact.
    ///
    /// Flip: if the method cleared ALL entries instead of just the named one,
    /// `"y"` would be absent and the second assertion would fire.  If it did
    /// nothing, `"x"` would still be present and the first assertion would fire.
    #[test]
    fn drop_activation_command_removes_entry_and_leaves_others() {
        use crate::ScriptingHost;

        let id = PluginId::parse("user/tp").unwrap();
        let id2 = PluginId::parse("user/other").unwrap();

        let mut host = ScriptingHost::new();
        // Seed two entries: only "x" will be dropped.
        host.registries
            .lazy_registry
            .activation_commands
            .insert("x".to_string(), id.clone());
        host.registries
            .cmd_owners
            .insert("x".to_string(), id.to_string());
        host.registries
            .lazy_registry
            .activation_commands
            .insert("y".to_string(), id2.clone());
        host.registries
            .cmd_owners
            .insert("y".to_string(), id2.to_string());

        host.drop_activation_command("x");

        assert!(
            !host.activation_commands().contains_key("x"),
            "activation_commands must not contain dropped entry 'x'"
        );
        assert!(
            !host.cmd_owners_for_test().contains_key("x"),
            "cmd_owners must not contain dropped entry 'x'"
        );
        // Unrelated entry must survive.
        assert!(
            host.activation_commands().contains_key("y"),
            "unrelated entry 'y' must not be removed"
        );
        assert!(
            host.cmd_owners_for_test().contains_key("y"),
            "unrelated cmd_owner 'y' must not be removed"
        );
    }

    // ── Windows path escaping ────────────────────────────────────────────────

    /// `%begin-lazy-activation` must escape backslashes in the plugin path so
    /// a Windows-style path (e.g. `C:\Users\x\plugin.scm`) survives embedding
    /// inside a Steel string literal without producing an invalid-escape error.
    ///
    /// This test inserts a synthetic backslash-bearing path without creating a
    /// real file; it checks only the returned require-string by comparing against
    /// the hand-computed expected value via Steel's `equal?`.
    ///
    /// Fail oracle: remove the `replace('\\', "\\\\")` call from
    /// `begin_lazy_activation` → the returned string is
    /// `(require "C:\Users\x\plugin.scm")` (raw backslashes) while the expected
    /// literal is `(require "C:\\Users\\x\\plugin.scm")` → `equal?` is `#f` →
    /// `error` fires → `eval_source` returns `Err` → `unwrap` panics.
    #[test]
    fn begin_lazy_activation_escapes_backslashes_in_path() {
        use crate::{ScriptingHost, null_host::NullHost};
        use std::path::PathBuf;

        let id = PluginId::parse("core:winpath").unwrap();
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry.plugins.insert(
            id.clone(),
            PluginState::Declared {
                path: PathBuf::from(r"C:\Users\x\plugin.scm"),
            },
        );

        // Oracle: each `\` in the path is doubled by the fix, so the Steel string
        // value held in `__result` is:
        //   (require "C:\\Users\\x\\plugin.scm")
        // To express that as a Steel string literal we double every `\` again:
        //   "(require \"C:\\\\Users\\\\x\\\\plugin.scm\")"
        // In a Rust raw string (r#"…"#) there is no further Rust escaping.
        let program = r#"
(define __result (%begin-lazy-activation "core:winpath"))
(when (not (equal? __result "(require \"C:\\\\Users\\\\x\\\\plugin.scm\")"))
  (error (string-append "backslash escaping wrong; got: " __result)))
"#;
        host.eval_source(program, &mut NullHost)
            .expect("backslashes in path must be doubled for Steel string embedding");

        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loading)
            ),
            "plugin must be Loading after %begin-lazy-activation"
        );
    }
}
