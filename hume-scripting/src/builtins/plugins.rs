//! Plugin lifecycle builtins: `%declare-plugin!`, `%load-plugin!`,
//! `resolve-plugin-path`, `declared-plugins`, `loaded-plugins`.
//!
//! `%declare-plugin!` backs the Scheme `declare-plugin` wrapper (lazy).
//! `%load-plugin!` backs the Scheme `load-plugin` wrapper (eager).
//! Both wrappers are defined in the bootstrap; see `builtins/mod.rs`.

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::{
    SteelCtx,
    attribution::{Owner, PluginId},
    lazy::PluginState,
};

use super::SteelResult;
use super::args::{list_items, list_to_strings};
use super::errors::generic_err;

// ── Helpers ───────────────────────────────────────────────────────────────────

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

/// Logs the "plugin file absent on disk" outcome, per plugin kind: `core:`
/// plugins go through [`log_absent_core`] (typo or broken `HUME_RUNTIME` —
/// never installed by PLUM); `user/repo` plugins log a softer Info (not yet
/// installed — PLUM will fetch it on `:plum-install`).
///
/// Shared by `declare_plugin` and `begin_manifest_declare`'s identical
/// absent-on-disk fork.
fn log_absent_plugin(ctx: &mut SteelCtx, plugin_id: &PluginId, name: &str, verb: &str) {
    match plugin_id {
        PluginId::Core(_) => log_absent_core(ctx, name, verb),
        PluginId::User { .. } => ctx.log(
            crate::log::LogLevel::Info,
            format!("{verb}: '{name}' not found on disk; install and reload to activate."),
        ),
    }
}

/// PLUM compat: records `name` in `declared_plugins` if not already present
/// (case-insensitive), regardless of whether the plugin resolves on disk —
/// PLUM reads this list to know what to install on `:plum-install`.
///
/// Shared by `declare_plugin`, `load_plugin`, and `begin_manifest_declare`.
fn record_declared(ctx: &mut SteelCtx, name: &str) {
    if !ctx
        .registries
        .declared_plugins
        .iter()
        .any(|d| d.eq_ignore_ascii_case(name))
    {
        ctx.registries.declared_plugins.push(name.to_string());
    }
}

/// Shared idempotency rule for both `declare-plugin` forms (regular and the
/// manifest zero-trigger fallback): `Loaded` → soft error, else first
/// declaration wins. Returns `true` if the caller should short-circuit
/// immediately (with its own success sentinel — `declare_plugin` and
/// `begin_manifest_declare` return different `SteelVal`s on this path).
fn already_declared(ctx: &mut SteelCtx, plugin_id: &PluginId, name: &str) -> bool {
    match ctx.registries.lazy_registry.plugins.get(plugin_id) {
        Some(PluginState::Loaded) => {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin: '{name}' is already loaded; ignoring declare"),
            );
            true
        }
        Some(_) => true, // Declared/Loading/Failed: first wins
        None => false,
    }
}

/// Builds `(require "<abs path>")`, rejecting a path containing `"` (which
/// can't be embedded in a Steel string literal) and escaping backslashes so
/// Windows paths (`C:\Users\…`) survive embedding — `\U` etc. are invalid
/// Steel escapes.
///
/// `kind` names what `path` is, for the error message (`"plugin"` /
/// `"plugin manifest"`). `on_unquotable` runs (for its side effect only)
/// before the shared error is raised — `begin_lazy_activation` uses it to
/// mark the plugin `Failed` first; `begin_manifest_declare` has no
/// plugin-stack state to unwind yet, so passes a no-op.
fn require_program_for_path(
    path: &std::path::Path,
    kind: &str,
    on_unquotable: impl FnOnce(),
) -> Result<String, SteelErr> {
    let abs_str = path.to_string_lossy();
    if abs_str.contains('"') {
        on_unquotable();
        steel::stop!(Generic =>
            "{} path contains '\"' — cannot embed in require: {}", kind, path.display());
    }
    let escaped = abs_str.replace('\\', "\\\\");
    Ok(format!("(require \"{escaped}\")"))
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

/// Error label for a `declare-plugin` keyword-argument decode.
///
/// Inside manifest resolution the offending code is the *plugin's*
/// `manifest.scm`, not the user's `init.scm` that the init-eval error prefix
/// will otherwise imply — so name it, and the plugin, explicitly.
fn declare_arg_label(ctx: &SteelCtx, keyword: &str) -> String {
    match &ctx.manifest_resolving {
        Some(id) => format!("declare-plugin {keyword} in manifest.scm for '{id}'"),
        None => format!("declare-plugin {keyword}"),
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
/// - Parses and validates activation entry lists *before* recording any
///   state, so a malformed entry leaves `declared_plugins`/`plugin_configs`
///   untouched. `#:events` entries are symbols, decoded and validated
///   against the host's `known_event_names()` via `hooks::event_name_arg`
///   (this crate has no compiled-in list of its own) — the same decoder
///   `register-hook!` uses, so the two verbs can't drift on accepted form.
///   `#:commands`/`#:languages` stay open strings.
/// - Stores `config` (the `#:config` value, first-wins) so the body can read
///   it back via `(plugin-config)` whenever activation eventually runs it.
/// - Records into `declared_plugins` for PLUM compat.
/// - Filters colliding command entries (logs `Severity::Error`, continues).
/// - Registers the plugin in `LazyRegistry`.
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

    if already_declared(ctx, &plugin_id, &name) {
        return Ok(SteelVal::Void);
    }

    // Decode and validate every activation-entry list before recording any state
    // below — a malformed entry must leave `declared_plugins`/`plugin_configs`
    // untouched, or PLUM would list a plugin the lazy registry never learns about.
    let cmd_list = list_to_strings(commands, &declare_arg_label(ctx, "#:commands"))?;
    let evt_label = declare_arg_label(ctx, "#:events");
    let evt_list: Vec<String> = list_items(events, &evt_label)?
        .iter()
        .map(|v| super::hooks::event_name_arg(ctx, v, &evt_label))
        .collect::<Result<_, _>>()?;
    let lang_list = list_to_strings(languages, &declare_arg_label(ctx, "#:languages"))?;

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

    record_declared(ctx, &name);

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
        log_absent_plugin(ctx, &plugin_id, &name, "declare-plugin");
        return Ok(SteelVal::Void);
    };

    // Filter colliding command names against the editor's live registry —
    // reached only now that the plugin is confirmed on disk. Each collision
    // logs a non-fatal Error (visible in :messages) and the name is dropped.
    // `register_lazy_command` claims the name in the same registry
    // `define-command!` and native commands live in, so this is the single
    // check for "is this name available" across builtin, activation, and
    // command-table names.
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
            .insert(cmd.clone(), Owner::Plugin(plugin_id.clone()));
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
    match std::fs::metadata(path) {
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
/// if already `Loaded`/`Failed`, `begin_lazy_activation`'s idempotency guard
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

    record_declared(ctx, &name);

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

/// Marks `id` `Failed` and runs the same cleanup `finish_lazy_activation`
/// would on failure — `drop_activations_for` (expired activation-event/language
/// entries) and `unregister_lazy_stubs_of` (dead `Lazy` command stub).
///
/// Shared by `begin_lazy_activation`'s two pre-body-eval raise paths (depth
/// limit, unquotable path) and `finish_lazy_activation`'s failure branch —
/// both leave a `Failed` plugin with no live activation footprint. Command/
/// hook rollback stays out of this helper: `begin_lazy_activation` raises
/// before the body ever runs, so no commands or hooks are registered under
/// this id yet.
fn fail_plugin_activation(ctx: &mut SteelCtx, id: &PluginId) {
    ctx.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Failed);
    ctx.registries.lazy_registry.drop_activations_for(id);
    ctx.host.commands().unregister_lazy_stubs_of(id);
}

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
        fail_plugin_activation(ctx, &id);
        steel::stop!(Generic =>
            "%begin-lazy-activation: activation depth limit ({}) exceeded — \
             check for circular load-plugin chains; '{}' marked Failed",
            MAX_ACTIVATION_DEPTH, id_str);
    }

    let require_program = require_program_for_path(&path, "plugin", || {
        fail_plugin_activation(ctx, &id);
    })?;

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
/// On failure, rolls back everything the partially-evaluated body
/// registered, so a `Failed` plugin leaves no live footprint: commands
/// (`define-command!`) are removed from `command_table`, `cmd_owners`, and
/// the editor's `CommandRegistry` (Steel globals defined before the error
/// stay in the VM's symbol table but are unreachable through HUME's
/// dispatch); hooks (`register-hook!`) tagged with this plugin's id are
/// dropped via `HookRegistry::remove_owned_by`; key bindings (`bind-key!` /
/// `bind-key-extend!` / `bind-wait-char!` / `unbind-key!`) queue an
/// `Effect::BindKey`/`BindWaitChar`/`UnbindKey` rather than mutating the
/// keymap inline, so `ctx.pop_effect_marks(success)` below drops a failed
/// body's binds along with everything else it queued — a plugin that would
/// have shadowed an existing binding and then fails leaves that binding
/// untouched, since the shadowing bind was never applied.
///
/// `ctx.pop_effect_marks(success)` does the same for every other queued side
/// effect (`register-lsp-server!`, `define-language!`, LSP requests, grammar
/// sweeps) — with one exception: an effect already committed by an
/// activation nested *inside* this body survives this failure too, since
/// that nested plugin's `Loaded` state is never rolled back either. See
/// `pop_effect_marks`.
///
/// Hooks can't use that committed-flag machinery: `HookRegistry` lives
/// inside `ScriptingHost`, but `Editor::init_scripting` holds the host in a
/// local until well after it applies every startup eval's effects — an
/// editor-applied `Effect::RegisterHook` would find `scripting == None` and
/// silently drop every hook `init.scm` registered. So `register-hook!`
/// mutates the persistent registry the instant the builtin runs, tagged
/// with a static owner set once at registration, and rollback removes
/// entries *by identity* (`owner == this id`) — a nested plugin's own
/// entries (a different owner) are never matched.
pub(crate) fn finish_lazy_activation(
    ctx: &mut SteelCtx,
    id_str: String,
    success: bool,
) -> SteelResult {
    let id = PluginId::parse(&id_str).map_err(generic_err)?;

    ctx.plugin_stack.pop();
    ctx.pop_effect_marks(success);

    if success {
        ctx.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loaded);
        ctx.registries.lazy_registry.drop_activations_for(&id);
        // Drop any `Lazy` stub the plugin didn't replace via `define-command!` —
        // dead weight now that the plugin is Loaded and won't re-run its body.
        ctx.host.commands().unregister_lazy_stubs_of(&id);
    } else {
        fail_plugin_activation(ctx, &id);
        // Roll back any commands the failed body partially registered.
        let owned_by_this_plugin = Owner::Plugin(id.clone());
        let orphans: Vec<String> = ctx
            .registries
            .cmd_owners
            .iter()
            .filter(|(_, owner)| **owner == owned_by_this_plugin)
            .map(|(name, _)| name.clone())
            .collect();
        for name in orphans {
            ctx.registries.command_table.remove(&name);
            ctx.registries.cmd_owners.remove(&name);
            ctx.host.commands().unregister_command(&name);
        }

        // Roll back hooks the failed body registered.
        ctx.registries.hooks.remove_owned_by(&id);
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

    if already_declared(ctx, &plugin_id, &name) {
        return Ok(SteelVal::BoolV(false));
    }

    // Recorded even when manifest resolution below can't find a file to
    // evaluate.
    record_declared(ctx, &name);

    let Some(dir) = plugin_dir_for_id(
        &plugin_id,
        ctx.dirs.runtime_dir.as_deref(),
        ctx.dirs.data_dir.as_deref(),
    ) else {
        return Ok(SteelVal::BoolV(false));
    };
    if !path_exists(&dir).map_err(generic_err)? {
        log_absent_plugin(ctx, &plugin_id, &name, "declare-plugin");
        return Ok(SteelVal::BoolV(false));
    }

    let manifest_path = dir.join("manifest.scm");
    if !path_exists(&manifest_path).map_err(generic_err)? {
        return Err(generic_err(format!(
            "declare-plugin: '{name}' has no manifest.scm; add #:commands/#:events/#:languages \
             to declare it explicitly, or use (load-plugin \"{name}\") for eager loading."
        )));
    }

    let require_program = require_program_for_path(&manifest_path, "plugin manifest", || {})?;

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
