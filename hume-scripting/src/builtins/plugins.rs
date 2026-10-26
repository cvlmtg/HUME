//! Plugin lifecycle builtins: `%declare-plugin!`, `%load-plugin!`,
//! `resolve-plugin-path`, `declared-plugins`, `declared-plugin-ref`,
//! `loaded-plugins`.
//!
//! `%declare-plugin!` backs the Scheme `declare-plugin!` wrapper: it declares
//! one lazy entry. `%load-plugin!` backs `load-plugin!`: a plugin with a
//! `manifest.scm` is lazy, one without loads eagerly. Both wrappers are defined
//! in the bootstrap; see `builtins/mod.rs`. The builtins that activate a
//! declared entry live in `activation`.

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::{
    Rooted, SteelCtx,
    attribution::{EntryFile, EntryId, PluginId},
    lazy::PluginState,
    plugin_record::{GitRef, LoadCall, Resolution},
};

use super::SteelResult;
use super::args::{list_items, list_to_strings, optional_string_arg};
use super::errors::generic_err;
use paths::{
    entry_path, installed_plugin_dir, path_exists, plugin_dir_for_id, require_program_for_path,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Log an `Error` for a `core:` plugin that is absent from the runtime dir.
///
/// For `core:` plugins, absent = typo or broken `HUME_RUNTIME`; PLUM never
/// installs them (they are bundled), so there is no "install and reload" path.
fn log_absent_core(ctx: &mut SteelCtx, name: &str, verb: &str) {
    ctx.log(
        crate::log::LogLevel::Error,
        format!(
            "{verb}: unknown core plugin '{name}': not found in runtime dir \
             (typo, or HUME_RUNTIME misconfigured)"
        ),
    );
}

/// Reports the "plugin file absent on disk" outcome, per plugin kind: `core:`
/// plugins go through [`log_absent_core`] (typo or broken `HUME_RUNTIME`,
/// never installed by PLUM); `user/repo` plugins log a softer Info (not yet
/// installed; PLUM will fetch it on `:plum-install-plugins`); a local file
/// has no install step, so its absence is an error.
fn absent_plugin(
    ctx: &mut SteelCtx,
    plugin_id: &PluginId,
    name: &str,
    verb: &str,
) -> Result<(), SteelErr> {
    match plugin_id {
        PluginId::Core(_) => log_absent_core(ctx, name, verb),
        PluginId::User { .. } => ctx.log(
            crate::log::LogLevel::Info,
            format!("{verb}: '{name}' not found on disk; install and reload to activate."),
        ),
        PluginId::Local(_) => {
            return Err(generic_err(format!(
                "local file '{name}' not found beside init.scm"
            )));
        }
    }
    Ok(())
}

/// Records that `plugin_id` is not installed and reports it, unless it was
/// already recorded as absent: both verbs and repeated calls share the one
/// report.
fn mark_absent(
    ctx: &mut SteelCtx,
    plugin_id: &PluginId,
    name: &str,
    verb: &str,
) -> Result<(), SteelErr> {
    let before = ctx
        .registries
        .plugin_records
        .resolve(plugin_id, Resolution::Absent)
        .map_err(generic_err)?;
    if before != Some(Resolution::Absent) {
        absent_plugin(ctx, plugin_id, name, verb)?;
    }
    Ok(())
}

/// Idempotency rule for `declare-plugin!`: `Loaded` → soft error, else first
/// declaration wins. Returns `true` if the caller should return immediately.
fn already_declared(ctx: &mut SteelCtx, entry: &EntryId, name: &str) -> bool {
    match ctx.registries.lazy_registry.plugins.get(entry) {
        Some(PluginState::Loaded) => {
            ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin!: '{name}' is already loaded; ignoring declare"),
            );
            true
        }
        Some(_) => true, // Declared/Loading/Failed: first wins
        None => false,
    }
}

/// Gate for plugin-registration verbs (`load-plugin!`, `declare-plugin!`).
///
/// Both verbs are valid only at the top level of `init.scm`, i.e. only
/// [`crate::context::EvalMode::Init`]. A plugin can never load or declare
/// another plugin: dependency declarations are the user's / plugin-manager's
/// responsibility, not a plugin's.
fn ensure_top_level(ctx: &SteelCtx) -> Result<(), SteelErr> {
    match ctx.mode() {
        crate::context::EvalMode::Init => Ok(()),
        crate::context::EvalMode::PluginLoad
        | crate::context::EvalMode::PluginActivation
        | crate::context::EvalMode::Command => {
            steel::stop!(Generic =>
                "can only be called at the top level of init.scm, not from a plugin body");
        }
    }
}

/// Error label for a `declare-plugin!` keyword-argument decode.
///
/// Inside manifest resolution the offending code is the *plugin's*
/// `manifest.scm`, not the user's `init.scm` that the init-eval error prefix
/// will otherwise imply, so name it, and the plugin, explicitly.
fn declare_arg_label(ctx: &SteelCtx, keyword: &str) -> String {
    match &ctx.manifest_resolving {
        Some(id) => format!("{keyword} in manifest.scm for '{id}'"),
        None => keyword.to_owned(),
    }
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// Whether any activation-trigger list is non-empty.
fn has_trigger(
    commands: &[String],
    typed_commands: &[String],
    events: &[String],
    languages: &[String],
) -> bool {
    !(commands.is_empty() && typed_commands.is_empty() && events.is_empty() && languages.is_empty())
}

/// Drops names that collide with a built-in and claims each remaining one as a
/// `Lazy` stub via `register` (`register_lazy_command` or
/// `register_lazy_typed_command`), logging a failed name instead of aborting.
fn filter_and_register_lazy(
    ctx: &mut SteelCtx,
    cmd_list: Vec<String>,
    entry: &EntryId,
    register: impl Fn(&mut dyn crate::host::CommandHost, &str, &EntryId) -> Result<(), String>,
) -> Vec<String> {
    let mut valid = Vec::with_capacity(cmd_list.len());
    for cmd in cmd_list {
        if ctx.builtin_cmd_names.contains(&cmd) {
            ctx.log(
                crate::log::LogLevel::Error,
                format!(
                    "declare-plugin!: command '{cmd}' conflicts with a built-in; activation entry ignored"
                ),
            );
            continue;
        }
        match register(ctx.host.commands(), &cmd, entry) {
            Ok(()) => valid.push(cmd),
            Err(msg) => ctx.log(
                crate::log::LogLevel::Error,
                format!("declare-plugin!: {msg}; activation entry ignored"),
            ),
        }
    }
    valid
}

/// `(%declare-plugin! name entry commands typed-commands events languages)`:
/// backs the Scheme `declare-plugin!` wrapper. Top-level only
/// (`ensure_top_level`), so a plugin can never declare another plugin.
///
/// `declare-plugin!` records one entry of a plugin (`entry` names its file,
/// `plugin.scm` by default) with its activation triggers and defers the
/// entry's body until one fires. A local plugin (`./file.scm`) is that one
/// file and takes no `entry`. At least one trigger is required, otherwise the
/// entry could never activate. Every trigger list is validated before any
/// state is recorded; `#:events` goes through `hooks::event_name_arg`, the
/// decoder `register-hook!` uses. A plugin's config comes from `load-plugin!`
/// only. Colliding command names are logged and skipped rather than failing
/// the declaration.
pub(crate) fn declare_plugin(
    ctx: &mut SteelCtx,
    name: String,
    entry: SteelVal,
    commands: SteelVal,
    typed_commands: SteelVal,
    events: SteelVal,
    languages: SteelVal,
) -> SteelResult {
    ensure_top_level(ctx)?;
    let plugin_id = PluginId::parse(&name).map_err(generic_err)?;
    let entry_file = entry_file_arg(entry)?;
    if plugin_id.is_local() && entry_file.is_some() {
        return Err(generic_err(format!(
            "'{name}' is a single file; #:entry applies to installed plugins"
        )));
    }
    let entry_id = EntryId::new(
        plugin_id.clone(),
        entry_file.unwrap_or_else(EntryFile::main),
    );

    // A manifest.scm being evaluated by load-plugin! may only declare the
    // plugin it belongs to. Otherwise a manifest for "foo/bar" could declare
    // an unrelated "baz/qux".
    if let Some(expected) = &ctx.manifest_resolving
        && *expected != plugin_id
    {
        return Err(generic_err(format!(
            "manifest.scm for '{expected}' must declare '{expected}', not '{name}'"
        )));
    }

    // A plugin that `load-plugin!` already ran for has had its manifest or
    // `plugin.scm` decide its entries. The manifest's own declare is the one
    // exception.
    if ctx.manifest_resolving.is_none() && ctx.registries.plugin_records.was_loaded(&plugin_id) {
        ctx.log(
            crate::log::LogLevel::Error,
            format!(
                "declare-plugin!: '{name}' comes after its load-plugin!; \
                 put declare-plugin! first. Declaration ignored."
            ),
        );
        return Ok(SteelVal::Void);
    }

    if already_declared(ctx, &entry_id, &name) {
        return Ok(SteelVal::Void);
    }

    // Decode and validate every activation-entry list before recording any state
    // below: a malformed entry must leave no plugin record behind, or PLUM
    // would list a plugin the lazy registry never learns about.
    let cmd_list = list_to_strings(commands, &declare_arg_label(ctx, "#:commands"))?;
    let typed_cmd_list =
        list_to_strings(typed_commands, &declare_arg_label(ctx, "#:typed-commands"))?;
    let evt_label = declare_arg_label(ctx, "#:events");
    let evt_list: Vec<String> = list_items(events, &evt_label)?
        .iter()
        .map(|v| super::hooks::event_name_arg(ctx, v, &evt_label))
        .collect::<Result<_, _>>()?;
    let lang_list = list_to_strings(languages, &declare_arg_label(ctx, "#:languages"))?;

    // Malformed name (not a collision) → hard error, same rules as
    // define-command!/define-typed-command!.  A name that can't survive
    // quoting is a typo, an editor command's name is lowercase, and a typed
    // name the `:` line can't read back whole could never run. This check is
    // independent of the plugin's on-disk path, so it runs regardless of
    // whether the plugin turns out to be absent.
    cmd_list
        .iter()
        .map(|cmd| crate::command_name::check_mappable(cmd))
        .chain(
            typed_cmd_list
                .iter()
                .map(|cmd| crate::command_name::check_typed(cmd)),
        )
        .collect::<Result<(), _>>()
        .map_err(generic_err)?;

    // Hard error: nothing declared at all. Checked against the raw (unfiltered)
    // lists, before path resolution: collision filtering only happens once the
    // plugin is confirmed present on disk (see below), so a non-empty
    // `cmd_list`/`typed_cmd_list` always skips this branch regardless of what
    // filtering later drops.
    if !has_trigger(&cmd_list, &typed_cmd_list, &evt_list, &lang_list) {
        return Err(generic_err(format!(
            "'{name}' declares no activation entries; it could never be activated. \
             Add #:commands/#:typed-commands/#:events/#:languages."
        )));
    }

    let path = entry_path(ctx.dirs, &entry_id).map_err(generic_err)?;

    // An entry naming a file that is missing from a plugin that is installed
    // is a misconfigured plugin, not a "not installed yet" state. A local file
    // has no entries; its absence is reported by `absent_plugin`.
    // Raised before anything is recorded so a failed declare leaves no trace.
    if path.is_none()
        && !plugin_id.is_local()
        && installed_plugin_dir(&plugin_id, ctx.dirs)
            .map_err(generic_err)?
            .is_some()
    {
        return Err(generic_err(format!(
            "'{name}' has no entry file '{}'",
            entry_id.file.as_str()
        )));
    }

    // When the plugin file is absent on disk, it can never be activated:
    // collision-checking (which claims the name in the editor's registry) would
    // be pointless and would leave the name claimed with no path to clean it up
    // via drop_activations_for's usual load/fail transition.  For user/ plugins,
    // log Info, since absent is expected before :plum-install-plugins.  For core: plugins,
    // absent means a typo or broken HUME_RUNTIME; PLUM never installs core:
    // plugins, so it can't catch the error.  The plugin is recorded here so
    // PLUM lists it.
    let Some(path) = path else {
        ctx.registries.plugin_records.note(plugin_id.clone(), None);
        mark_absent(ctx, &plugin_id, &name, "declare-plugin!")?;
        return Ok(SteelVal::Void);
    };

    // Filter colliding command names against the editor's live registry,
    // reached only now that the plugin is confirmed on disk. Each collision
    // logs a non-fatal Error (visible in :messages) and the name is dropped.
    // `register_lazy_command`/`register_lazy_typed_command` claims the name
    // in the same registry `define-command!`/`define-typed-command!` and
    // native commands live in, so this is the single check for "is this name
    // available" across builtin, activation, and Steel-defined names.
    let cmd_list = filter_and_register_lazy(ctx, cmd_list, &entry_id, |c, name, entry| {
        c.register_lazy_command(name, entry)
    });
    let typed_cmd_list =
        filter_and_register_lazy(ctx, typed_cmd_list, &entry_id, |c, name, entry| {
            c.register_lazy_typed_command(name, entry)
        });

    // Hard error: all supplied #:commands/#:typed-commands entries collided,
    // and no #:events/#:languages entries either: the plugin has no usable
    // activation entry left. The all-empty case already returned above, so
    // reaching here means at least one list was non-empty before filtering, so
    // the message always names the collision, never "none were supplied".
    if !has_trigger(&cmd_list, &typed_cmd_list, &evt_list, &lang_list) {
        return Err(generic_err(format!(
            "'{name}' declares no activation entries; \
             all #:commands/#:typed-commands entries conflicted with existing commands. \
             Fix the collision."
        )));
    }

    ctx.registries.plugin_records.note(plugin_id.clone(), None);

    ctx.registries
        .lazy_registry
        .declare(entry_id, path, evt_list, lang_list);

    Ok(SteelVal::Void)
}

/// `(resolve-plugin-path name)`: return the resolved path string if the
/// plugin file exists on disk, or `#f` if absent.  Raises a Steel error for
/// malformed names.
pub(crate) fn resolve_plugin_path(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let plugin_id = PluginId::parse_installed(&name).map_err(generic_err)?;
    let path = entry_path(ctx.dirs, &EntryId::main(plugin_id)).map_err(generic_err)?;
    match path {
        Some(p) => Ok(SteelVal::StringV(p.to_string_lossy().into_owned().into())),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(%load-plugin! name config ref)`: Rust primitive backing the Scheme
/// `load-plugin!` wrapper.
///
/// Top-level only (`ensure_top_level`): a plugin can never load another
/// plugin, and a manifest being evaluated cannot call it.
///
/// Stores `config` and `ref` unconditionally, replacing any earlier ones, and records the
/// plugin for PLUM. A plugin that already has a declared entry (declared by
/// the user, loaded before, or evaluated through its manifest) is left as it
/// is. Otherwise the plugin's directory decides: a `manifest.scm` makes it
/// lazy and the returned `(require "<abs manifest.scm>")` string is evaluated
/// by the wrapper, which hands the outcome to `%finish-manifest-load!`; with
/// only a `plugin.scm` the main entry is declared and `#t` asks the wrapper to
/// activate it now. `#f` means nothing is left to do. A directory with neither
/// file is an error.
pub(crate) fn load_plugin(
    ctx: &mut SteelCtx,
    name: String,
    config: SteelVal,
    git_ref: SteelVal,
) -> SteelResult {
    ensure_top_level(ctx)?;
    if ctx.manifest_resolving.is_some() {
        steel::stop!(Generic =>
            "cannot be called from a manifest.scm; a manifest declares its own \
             plugin with declare-plugin!");
    }
    let plugin_id = PluginId::parse_installed(&name).map_err(generic_err)?;
    let git_ref = git_ref_arg(&plugin_id, git_ref)?;

    ctx.registries.plugin_records.note(
        plugin_id.clone(),
        Some(LoadCall {
            config: Rooted::new(config),
            git_ref,
        }),
    );

    if ctx
        .registries
        .plugin_records
        .resolution(&plugin_id)
        .is_some()
        || ctx.registries.lazy_registry.declares_plugin(&plugin_id)
    {
        return Ok(SteelVal::BoolV(false));
    }
    let Some(dir) = installed_plugin_dir(&plugin_id, ctx.dirs).map_err(generic_err)? else {
        mark_absent(ctx, &plugin_id, &name, "load-plugin!")?;
        return Ok(SteelVal::BoolV(false));
    };

    let manifest_path = dir.join("manifest.scm");
    if path_exists(&manifest_path).map_err(generic_err)? {
        let require_program = require_program_for_path(&manifest_path, "plugin manifest", || {})?;
        ctx.manifest_resolving = Some(plugin_id);
        // Mirrors `begin_lazy_activation`'s own mark: anything manifest.scm
        // queues (`register-lsp-server!`, a nested activation, …) rolls back
        // with the rest of a failed evaluation (see `finish_manifest_load`).
        ctx.mark_effects();
        return Ok(SteelVal::StringV(require_program.into()));
    }

    let main = EntryId::main(plugin_id.clone());
    if let Some(path) = entry_path(ctx.dirs, &main).map_err(generic_err)? {
        ctx.registries
            .lazy_registry
            .declare(main, path, Vec::new(), Vec::new());
        return Ok(SteelVal::BoolV(true));
    }
    Err(generic_err(format!(
        "'{name}' has neither manifest.scm nor plugin.scm in {}",
        dir.display()
    )))
}

/// `load-plugin!`'s `#:ref`: `#f`, or the tag, branch or commit PLUM checks out.
fn git_ref_arg(plugin_id: &PluginId, git_ref: SteelVal) -> Result<Option<GitRef>, SteelErr> {
    let Some(name) = optional_string_arg(git_ref, "#:ref")? else {
        return Ok(None);
    };
    let Some(git_ref) = GitRef::parse(name.clone()) else {
        steel::stop!(Generic => "#:ref \"{}\" is not a tag, branch or commit", name);
    };
    if matches!(plugin_id, PluginId::Core(_)) {
        steel::stop!(Generic =>
            "#:ref is for an installed plugin; '{}' is bundled with HUME", plugin_id);
    }
    Ok(Some(git_ref))
}

/// Decodes an entry-file argument: `#f` is `None` (the main entry by
/// default), a string names that file.
fn entry_file_arg(entry: SteelVal) -> Result<Option<EntryFile>, SteelErr> {
    optional_string_arg(entry, "plugin entry")?
        .map(|file| EntryFile::parse(&file).map_err(generic_err))
        .transpose()
}

/// `(loaded-plugins)`: return a Steel list of plugin names whose `plugin.scm`
/// entry is in `Loaded` state.
///
/// Derived from `LazyRegistry` so lazy plugins correctly read as not-yet-loaded
/// until their body has been evaluated.
pub(crate) fn loaded_plugins(ctx: &mut SteelCtx) -> SteelResult {
    let vals: Vec<SteelVal> = ctx
        .registries
        .lazy_registry
        .plugins
        .iter()
        .filter(|(id, state)| id.file.is_main() && matches!(state, PluginState::Loaded))
        .map(|(id, _)| SteelVal::StringV(id.plugin.to_string().into()))
        .collect();
    vals.into_steelval().map_err(generic_err)
}

/// `(declared-plugins)`: return a Steel list of every declared plugin name,
/// `core:*` included.  PLUM filters out `core:*` itself where install policy
/// requires it (core plugins are bundled, never installed by PLUM).
pub(crate) fn declared_plugins(ctx: &mut SteelCtx) -> SteelResult {
    let vals: Vec<SteelVal> = ctx
        .registries
        .plugin_records
        .plum_names()
        .map(|s| SteelVal::StringV(s.into()))
        .collect();
    vals.into_steelval().map_err(generic_err)
}

/// `(declared-plugin-ref name)`: the `#:ref` the last `load-plugin!` of
/// `name` passed, or `#f`. Raises for a malformed name.
pub(crate) fn declared_plugin_ref(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let plugin_id = PluginId::parse_installed(&name).map_err(generic_err)?;
    Ok(match ctx.registries.plugin_records.git_ref(&plugin_id) {
        Some(git_ref) => SteelVal::StringV(git_ref.into()),
        None => SteelVal::BoolV(false),
    })
}

/// Empty Steel hash: the `(plugin-config)` default when no config was passed.
fn empty_config() -> SteelResult {
    std::collections::HashMap::<String, SteelVal>::new()
        .into_steelval()
        .map_err(generic_err)
}

/// `(plugin-config)`: return the calling plugin's `#:config` value, or an
/// empty hash if none was passed (or if called outside a plugin body).
///
/// Resolved via the top of `plugin_stack`, which is non-empty for the whole
/// duration of a plugin body's evaluation, pushed in `begin_lazy_activation`
/// before either `load-plugin!` (eager) or a deferred lazy activation runs the
/// `(require …)`. Both paths therefore read config identically.
pub(crate) fn plugin_config(ctx: &mut SteelCtx) -> SteelResult {
    let Some(id) = ctx.plugin_stack.current() else {
        return empty_config();
    };
    match ctx.registries.plugin_records.config(&id.plugin) {
        Some(cfg) => Ok(cfg.clone()),
        None => empty_config(),
    }
}

/// `(plugin-dir)`: the directory holding the calling plugin's own files, or
/// `#f` outside a plugin body (or when the root for that plugin's kind is
/// unset). Resolved from the top of `plugin_stack` like `(plugin-config)`.
pub(crate) fn plugin_dir(ctx: &mut SteelCtx) -> SteelResult {
    let dir = ctx
        .plugin_stack
        .current()
        .and_then(|id| plugin_dir_for_id(&id.plugin, ctx.dirs));
    Ok(match dir {
        Some(d) => SteelVal::StringV(d.to_string_lossy().into_owned().into()),
        None => SteelVal::BoolV(false),
    })
}

pub(super) mod activation;
pub(crate) use activation::abandon_open_activations;
pub(crate) mod paths;

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// Parsing tests (valid/invalid plugin names, segments) live in
// `hume_scripting::attribution::tests` alongside `PluginId::parse`.  The tests here
// cover only the builtins' Steel-facing behaviour.

#[cfg(test)]
mod tests;
