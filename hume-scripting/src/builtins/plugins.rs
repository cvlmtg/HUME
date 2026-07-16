//! Plugin lifecycle builtins: `%declare-plugin!`, `%load-plugin!`,
//! `resolve-plugin-path`, `declared-plugins`, `loaded-plugins`.
//!
//! `%declare-plugin!` backs the Scheme `declare-plugin` wrapper (lazy).
//! `%load-plugin!` backs the Scheme `load-plugin` wrapper (eager).
//! Both wrappers are defined in the bootstrap; see `builtins/mod.rs`.

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::{SteelCtx, attribution::PluginId, hooks::HookId, lazy::PluginState};

use super::args::list_to_strings;
use super::errors::generic_err;

// ── Helpers ───────────────────────────────────────────────────────────────────

type SteelResult = Result<SteelVal, SteelErr>;

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
/// Both verbs are valid only at the top level of `init.scm` — i.e. only
/// [`crate::context::EvalMode::Init`]. A plugin can never load or declare
/// another plugin — dependency declarations are the user's / plugin-manager's
/// responsibility, not a plugin's.
fn ensure_top_level(ctx: &SteelCtx, verb: &str) -> Result<(), SteelErr> {
    match ctx.mode() {
        crate::context::EvalMode::Init => Ok(()),
        crate::context::EvalMode::PluginLoad
        | crate::context::EvalMode::PluginActivation
        | crate::context::EvalMode::Command => {
            steel::stop!(Generic =>
                "{}: can only be called at the top level of init.scm, not from a plugin body",
                verb);
        }
    }
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(%declare-plugin! name commands events languages config)` — Rust primitive
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
/// - Stores `config` (the `#:config` value, first-wins) so the body can read it
///   back via `(plugin-config)` whenever activation eventually runs it.
pub(crate) fn declare_plugin(
    ctx: &mut SteelCtx,
    name: String,
    commands: SteelVal,
    events: SteelVal,
    languages: SteelVal,
    config: SteelVal,
) -> SteelResult {
    ensure_top_level(ctx, "declare-plugin")?;
    let plugin_id = PluginId::parse(&name).map_err(generic_err)?;

    // A manifest.scm being resolved by %begin-manifest-declare! may only ever
    // declare the plugin it was resolved for — otherwise a manifest for
    // "foo/bar" could smuggle in an unrelated "baz/qux" declaration under the
    // same zero-trigger gate.
    if let Some(expected) = &ctx.manifest_resolving
        && *expected != plugin_id
    {
        return Err(generic_err(format!(
            "declare-plugin: manifest.scm for '{expected}' must declare '{expected}', not '{name}'"
        )));
    }

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

    // First declaration wins for config too, matching the state no-op above:
    // stored now so it is already in place by the time activation runs the body.
    // Inside manifest resolution, the user's #:config was already stored by
    // %begin-manifest-declare! before manifest.scm ran — that must win over the
    // manifest's own default, so use or_insert instead of an unconditional overwrite.
    if ctx.manifest_resolving.is_some() {
        ctx.registries
            .plugin_configs
            .entry(plugin_id.clone())
            .or_insert(config);
    } else {
        ctx.registries
            .plugin_configs
            .insert(plugin_id.clone(), config);
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

    // Malformed name (not a collision) → hard error, same rule as
    // define-command!.  A name that can't survive quoting is a typo — this
    // check is independent of the plugin's on-disk path, so it runs
    // regardless of whether the plugin turns out to be absent.
    for cmd in &cmd_list {
        if cmd.contains('"') || cmd.contains('\\') {
            steel::stop!(Generic =>
                "declare-plugin: command name '{}' must not contain '\"' or '\\'", cmd);
        }
    }

    let evt_list: Vec<HookId> = evt_strs
        .iter()
        .map(|s| {
            HookId::from_symbol(s).ok_or_else(|| {
                let valid = HookId::all_names().collect::<Vec<_>>().join(", ");
                generic_err(format!("events: unknown hook '{}'; valid: {}", s, valid))
            })
        })
        .collect::<Result<_, _>>()?;

    // Hard error: nothing declared at all. Checked against the raw (unfiltered)
    // lists, before path resolution — collision filtering only happens once the
    // plugin is confirmed present on disk (see below), so a non-empty
    // `cmd_list` always skips this branch regardless of what filtering later
    // drops.
    if cmd_list.is_empty() && evt_list.is_empty() && lang_list.is_empty() {
        return Err(generic_err(format!(
            "declare-plugin: '{name}' declares no activation entries; it could never be activated. \
             Add #:commands/#:events/#:languages, or use (load-plugin \"{name}\") for eager loading."
        )));
    }

    let path = resolve_path_for_name(
        &name,
        ctx.dirs.runtime_dir.as_deref(),
        ctx.dirs.data_dir.as_deref(),
    )
    .map_err(generic_err)?;

    // When the plugin file is absent on disk, it can never be activated —
    // collision-checking (which claims the name in the editor's registry) would
    // be pointless and would leave the name claimed with no path to clean it up
    // via drop_activations_for's usual load/fail transition.  For user/ plugins,
    // log Info — absent is expected before :plum-install.  For core: plugins,
    // absent means a typo or broken HUME_RUNTIME; PLUM never installs core:
    // plugins, so it can't catch the error.  `declared_plugins` is already
    // recorded above for PLUM.
    let Some(path) = path else {
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
    };

    // Filter colliding command names against the editor's live registry —
    // reached only now that the plugin is confirmed on disk. Each collision
    // logs a non-fatal Error (visible in :messages) and the name is dropped.
    // `register_lazy_command` claims the name in the same registry
    // `define-command!` and native commands live in, so this is the single
    // check for "is this name available", replacing the old three-way
    // builtin/activation_commands/command_table lookup.
    let mut valid = Vec::with_capacity(cmd_list.len());
    for cmd in cmd_list {
        if ctx.builtin_cmd_names.contains(&cmd) {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: command '{cmd}' conflicts with a built-in; activation entry ignored"),
            );
            continue;
        }
        match ctx.host.commands().register_lazy_command(&cmd, &plugin_id) {
            Ok(()) => valid.push(cmd),
            Err(msg) => ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: {msg}; activation entry ignored"),
            ),
        }
    }
    let cmd_list = valid;

    // Hard error: all supplied #:commands entries collided, and no #:events/
    // #:languages entries either — the plugin has no usable activation entry
    // left. The all-empty case already returned above, so reaching here means
    // `cmd_list` was non-empty before filtering — the message always names
    // the collision, never "none were supplied".
    if cmd_list.is_empty() && evt_list.is_empty() && lang_list.is_empty() {
        return Err(generic_err(format!(
            "declare-plugin: '{name}' declares no activation entries; \
             all #:commands entries conflicted with existing commands. \
             Fix the collision or use (load-plugin \"{name}\") for eager loading."
        )));
    }

    // Pre-seed cmd_owners so (command-plugin "cmd") resolves correctly before
    // the plugin body is evaluated (before activation).  Only for accepted
    // names — a filtered-out collision must not gain attribution here.
    for cmd in &cmd_list {
        ctx.registries
            .cmd_owners
            .insert(cmd.clone(), plugin_id.to_string());
    }

    ctx.registries
        .lazy_registry
        .declare(plugin_id, Some(path), evt_list, lang_list);

    Ok(SteelVal::Void)
}

/// The directory a plugin's files live in, given its id — `core:` plugins
/// under `runtime_dir`, `user/repo` plugins under `data_dir`. `None` when the
/// relevant root is unset (`HOME`/`APPDATA` unset for user plugins).
///
/// Shared by `plugin.scm` resolution (`resolve_path_for_name`) and
/// `manifest.scm` resolution (`begin_manifest_declare`) so the two-root
/// layout logic lives in one place.
fn plugin_dir_for_id(
    plugin_id: &PluginId,
    runtime_dir: Option<&std::path::Path>,
    data_dir: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    match plugin_id {
        PluginId::Core(core_name) => {
            runtime_dir.map(|rt| rt.join("plugins").join("core").join(core_name))
        }
        // When data_dir is None (HOME/APPDATA unset), user plugins cannot be
        // resolved — return None rather than panicking.
        PluginId::User { user, repo } => data_dir.map(|d| d.join("plugins").join(user).join(repo)),
    }
}

/// Probe a path's existence without a pre-flight `.exists()` (avoids TOCTOU).
/// `NotFound` → `Ok(false)`; other errors propagate.
fn path_exists(path: &std::path::Path) -> Result<bool, String> {
    match hume_platform::fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("cannot stat path '{}': {e}", path.display())),
    }
}

/// Pure path resolution: given a plugin name and the runtime / data directories,
/// return the resolved `PathBuf` if `plugin.scm` exists on disk, or `None`.
///
/// Called by the `resolve-plugin-path` Steel builtin (which accesses the dirs
/// via `&mut SteelCtx`).
pub(crate) fn resolve_path_for_name(
    name: &str,
    runtime_dir: Option<&std::path::Path>,
    data_dir: Option<&std::path::Path>,
) -> Result<Option<std::path::PathBuf>, String> {
    let plugin_id = PluginId::parse(name)?;
    let Some(dir) = plugin_dir_for_id(&plugin_id, runtime_dir, data_dir) else {
        return Ok(None);
    };
    let file = dir.join("plugin.scm");
    if path_exists(&file)? {
        Ok(Some(file))
    } else {
        Ok(None)
    }
}

/// `(resolve-plugin-path name)` — return the resolved path string if the
/// plugin file exists on disk, or `#f` if absent.  Raises a Steel error for
/// malformed names.
pub(crate) fn resolve_plugin_path(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let path = resolve_path_for_name(
        &name,
        ctx.dirs.runtime_dir.as_deref(),
        ctx.dirs.data_dir.as_deref(),
    )
    .map_err(generic_err)?;
    match path {
        Some(p) => Ok(SteelVal::StringV(p.to_string_lossy().into_owned().into())),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(%load-plugin! "name" config)` — Rust primitive backing the Scheme-side
/// `load-plugin` wrapper (eager).
///
/// Top-level only: a plugin can never load another plugin — see
/// `ensure_top_level`.
///
/// Stores `config` unconditionally, overriding any prior value (unlike
/// `declare-plugin`'s first-wins) — a bare `(load-plugin "x")` after
/// `(declare-plugin "x" #:config h)` runs the body with the empty default,
/// not `h`, since the most recent call should always win. Read back by the
/// body via `(plugin-config)`.
///
/// If not yet declared, resolves its path and registers it now: absent on
/// disk → silent skip + record in `declared_plugins` for PLUM to install on
/// the next `:plum-install`. If already declared, queues it for activation;
/// if already `Loaded`/`Failed`, `activate_plugin`'s idempotency guard
/// no-ops it.
pub(crate) fn load_plugin(ctx: &mut SteelCtx, name: String, config: SteelVal) -> SteelResult {
    ensure_top_level(ctx, "load-plugin")?;
    let id = PluginId::parse(&name).map_err(generic_err)?;

    // load-plugin always overrides: unlike declare-plugin's first-wins, a repeat
    // (re)load intentionally replaces the config the body will see next activation.
    ctx.registries.plugin_configs.insert(id.clone(), config);

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
        let path = resolve_path_for_name(
            &name,
            ctx.dirs.runtime_dir.as_deref(),
            ctx.dirs.data_dir.as_deref(),
        )
        .map_err(generic_err)?;
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
    let id = PluginId::parse(&id_str).map_err(generic_err)?;

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
    ctx.mark_effects();

    Ok(SteelVal::StringV(require_program.into()))
}

/// `(%finish-lazy-activation id-str success?)` — Rust primitive for inline
/// activation. Called from `%activate-plugin-inline` after
/// `(hm.eval-string …)` completes or fails. Pops `plugin_stack` and
/// transitions the plugin to `Loaded`/`Failed`; `drop_activations_for` runs
/// on both paths to clean up expired activation entries.
///
/// On failure, rolls back everything the partially-evaluated body registered,
/// so a `Failed` plugin leaves no live footprint:
///   - commands (`define-command!`): removed from `command_table`,
///     `cmd_owners`, and the editor's `CommandRegistry` — Steel globals
///     defined before the error stay in the VM's symbol table but are
///     unreachable through HUME's dispatch;
///   - hooks (`register-hook!`): every handler tagged with this plugin's id
///     is dropped via `HookRegistry::remove_owned_by`, so a `Failed`
///     plugin's hooks stop firing;
///   - key bindings (`bind-key!` / `bind-key-extend!` / `bind-wait-char!`):
///     every key this plugin bound is unbound via the `key_bindings` ledger
///     and `KeymapHost::unbind_key`. This *unbinds* rather than *restores*
///     whatever the key was bound to before — `bind-key!` overwrites
///     silently with no collision detection (rebinding is a legitimate,
///     common operation), so there is no prior binding to snapshot without
///     much heavier machinery. A plugin that shadows an existing key and
///     then fails leaves that key unbound, not reverted. Accepted scope.
///
/// `ctx.pop_effect_marks(success)` does the same for every queued side effect
/// (`register-lsp-server!`, `define-language!`, LSP requests, grammar
/// sweeps) queued via `mark_effects` — with one exception: an effect already
/// committed by an activation nested *inside* this body (i.e. this body
/// itself called into another plugin's inline activation, which finished
/// successfully) survives this failure too, because that nested plugin's
/// `Loaded` state is never rolled back either. See `pop_effect_marks`.
///
/// Hooks and key bindings need none of that committed-flag machinery: they
/// mutate persistent registries immediately and permanently the instant the
/// builtin runs, each tagged with a static owner set once at registration —
/// rollback removes entries *by identity* (`owner == this id`), never by
/// eval-scoped position, so a nested plugin's own entries (a different
/// owner) are simply never matched.
pub(crate) fn finish_lazy_activation(
    ctx: &mut SteelCtx,
    id_str: String,
    success: bool,
) -> SteelResult {
    let id = PluginId::parse(&id_str).map_err(generic_err)?;

    ctx.plugin_stack.pop();
    ctx.pop_effect_marks(success);

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
    // Drop any `Lazy` stub the plugin didn't replace via `define-command!` —
    // on success, dead weight (the plugin is Loaded and won't re-run its
    // body); on failure, frees the name for a later plugin to claim.
    ctx.host.commands().unregister_lazy_stubs_of(&id);

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
            ctx.host.commands().unregister_command(&name);
        }

        // Roll back hooks the failed body registered.
        ctx.registries.hooks.remove_owned_by(&id);

        // Roll back key bindings the failed body made.
        let orphan_binds: Vec<_> = ctx
            .registries
            .key_bindings
            .iter()
            .filter(|(owner, _, _)| owner == &id)
            .map(|(_, mode, keys)| (*mode, keys.clone()))
            .collect();
        ctx.registries
            .key_bindings
            .retain(|(owner, _, _)| owner != &id);
        for (mode, keys) in orphan_binds {
            let _ = ctx.host.keymap().unbind_key(mode, &keys);
        }
    }

    Ok(SteelVal::Void)
}

/// `(%lazy-command-owner name)` — return the owning plugin's id string if `name`
/// is a registered activation command, or `#f` if not.  Used by `%dispatch-command`
/// to decide whether a `command_table` miss should trigger inline activation.
pub(crate) fn lazy_command_owner(ctx: &mut SteelCtx, name: String) -> SteelResult {
    match ctx.host.commands().lazy_command_owner(&name) {
        Some(id) => Ok(SteelVal::StringV(id.to_string().into())),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(%begin-manifest-declare! name config)` — Rust primitive backing the
/// zero-trigger branch of the Scheme `declare-plugin` wrapper.
///
/// A `(declare-plugin "id")` call with no `#:commands`/`#:events`/`#:languages`
/// is routed here instead of `%declare-plugin!`: rather than hard-erroring,
/// HUME looks for `<plugin-dir>/manifest.scm` and evaluates it so the plugin
/// can declare itself with its own default activation triggers.
///
/// Returns the `(require "<abs manifest.scm>")` string to eval (mirrors
/// `%begin-lazy-activation`), or `#f` for the no-op cases: already declared
/// (first-wins) or absent on disk (soft-logged exactly like `%declare-plugin!`
/// — a user plugin not yet installed by PLUM, or a core plugin typo/broken
/// `HUME_RUNTIME`). Hard-errors when the plugin directory exists but has no
/// `manifest.scm` (a misconfigured plugin, not a "not installed yet" state),
/// and when a manifest resolution is already in progress — a manifest whose
/// own self-declare is itself zero-trigger would otherwise recurse forever.
pub(crate) fn begin_manifest_declare(
    ctx: &mut SteelCtx,
    name: String,
    config: SteelVal,
) -> SteelResult {
    ensure_top_level(ctx, "declare-plugin")?;

    if ctx.manifest_resolving.is_some() {
        steel::stop!(Generic =>
            "declare-plugin: '{}' has no activation entries and manifest.scm \
             must declare at least one — a manifest.scm cannot itself be zero-trigger",
            name);
    }

    let plugin_id = PluginId::parse(&name).map_err(generic_err)?;

    // Same idempotency rule as %declare-plugin!: Loaded → soft error, else first wins.
    match ctx.registries.lazy_registry.plugins.get(&plugin_id) {
        Some(PluginState::Loaded) => {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: '{name}' is already loaded; ignoring declare"),
            );
            return Ok(SteelVal::BoolV(false));
        }
        Some(_) => return Ok(SteelVal::BoolV(false)), // Declared/Loading/Failed: first wins
        None => {}
    }

    // PLUM compat: declared_plugins always records every declared plugin, even
    // when manifest resolution below can't find a file to evaluate.
    if !ctx
        .registries
        .declared_plugins
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&name))
    {
        ctx.registries.declared_plugins.push(name.clone());
    }

    let Some(dir) = plugin_dir_for_id(
        &plugin_id,
        ctx.dirs.runtime_dir.as_deref(),
        ctx.dirs.data_dir.as_deref(),
    ) else {
        return Ok(SteelVal::BoolV(false));
    };
    if !path_exists(&dir).map_err(generic_err)? {
        match &plugin_id {
            PluginId::Core(_) => log_absent_core(ctx, &name, "declare-plugin"),
            PluginId::User { .. } => ctx.log(
                crate::log::LogLevel::Info,
                format!(
                    "declare-plugin: '{name}' not found on disk; install and reload to activate."
                ),
            ),
        }
        return Ok(SteelVal::BoolV(false));
    }

    let manifest_path = dir.join("manifest.scm");
    if !path_exists(&manifest_path).map_err(generic_err)? {
        return Err(generic_err(format!(
            "declare-plugin: '{name}' has no manifest.scm; add #:commands/#:events/#:languages \
             to declare it explicitly, or use (load-plugin \"{name}\") for eager loading."
        )));
    }

    let abs_str = manifest_path.to_string_lossy();
    if abs_str.contains('"') {
        return Err(generic_err(format!(
            "plugin manifest path contains '\"' — cannot embed in require: {}",
            manifest_path.display()
        )));
    }
    // Escape backslashes so Windows paths survive embedding in a Steel string literal.
    let escaped = abs_str.replace('\\', "\\\\");
    let require_program = format!("(require \"{escaped}\")");

    // Store the user's #:config now, before evaluating manifest.scm, so the
    // or_insert guard in declare_plugin (fired by the manifest's own
    // declare-plugin call) doesn't let the manifest's default clobber it.
    ctx.registries
        .plugin_configs
        .insert(plugin_id.clone(), config);
    ctx.manifest_resolving = Some(plugin_id);

    Ok(SteelVal::StringV(require_program.into()))
}

/// `(%finish-manifest-declare! name success?)` — Rust primitive; the tail half
/// of the zero-trigger `declare-plugin` path (mirrors `%finish-lazy-activation`).
///
/// Clears `manifest_resolving` unconditionally. On success, verifies the
/// manifest actually declared the plugin — a `manifest.scm` that evaluates
/// without error but never calls `declare-plugin` would otherwise leave the
/// plugin silently undeclared.
pub(crate) fn finish_manifest_declare(
    ctx: &mut SteelCtx,
    name: String,
    success: bool,
) -> SteelResult {
    let id = PluginId::parse(&name).map_err(generic_err)?;
    ctx.manifest_resolving = None;

    if success && !ctx.registries.lazy_registry.plugins.contains_key(&id) {
        return Err(generic_err(format!(
            "declare-plugin: manifest.scm for '{name}' did not declare '{name}' — a \
             manifest.scm must call (declare-plugin \"{name}\" …) with at least one \
             activation entry"
        )));
    }

    Ok(SteelVal::Void)
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
    vals.into_steelval().map_err(generic_err)
}

/// `(declared-plugins)` — return a Steel list of every declared plugin name,
/// `core:*` included.  PLUM filters out `core:*` itself where install policy
/// requires it (core plugins are bundled, never installed by PLUM).
pub(crate) fn declared_plugins(ctx: &mut SteelCtx) -> SteelResult {
    let vals: Vec<SteelVal> = ctx
        .registries
        .declared_plugins
        .iter()
        .map(|s| SteelVal::StringV(s.as_str().into()))
        .collect();
    vals.into_steelval().map_err(generic_err)
}

/// Empty Steel hash — the `(plugin-config)` default when no config was passed.
fn empty_config() -> SteelResult {
    std::collections::HashMap::<String, SteelVal>::new()
        .into_steelval()
        .map_err(generic_err)
}

/// `(plugin-config)` — return the calling plugin's `#:config` value, or an
/// empty hash if none was passed (or if called outside a plugin body).
///
/// Resolved via the top of `plugin_stack`, which is non-empty for the whole
/// duration of a plugin body's evaluation — pushed in `begin_lazy_activation`
/// before either `load-plugin` (eager) or a deferred lazy activation runs the
/// `(require …)`. Both paths therefore read config identically.
pub(crate) fn plugin_config(ctx: &mut SteelCtx) -> SteelResult {
    let Some(id) = ctx.plugin_stack.current() else {
        return empty_config();
    };
    match ctx.registries.plugin_configs.get(id) {
        Some(cfg) => Ok(cfg.clone()),
        None => empty_config(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// Parsing tests (valid/invalid plugin names, segments) live in
// `hume_scripting::attribution::tests` alongside `PluginId::parse`.  The tests here
// cover only the builtins' Steel-facing behaviour.

#[cfg(test)]
mod tests;
