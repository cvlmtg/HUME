//! Plugin lifecycle builtins: `%declare-plugin!`, `%load-plugin!`,
//! `resolve-plugin-path`, `declared-plugins`, `loaded-plugins`.
//!
//! `%declare-plugin!` backs the Scheme `declare-plugin!` wrapper: it declares
//! one lazy entry. `%load-plugin!` backs `load-plugin!`: a plugin with a
//! `manifest.scm` is lazy, one without loads eagerly. Both wrappers are defined
//! in the bootstrap; see `builtins/mod.rs`.

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::{
    SteelCtx,
    attribution::{EntryFile, EntryId, Owner, PluginId},
    lazy::PluginState,
};

use super::SteelResult;
use super::args::{list_items, list_to_strings, optional_steel_error_arg, optional_string_arg};
use super::dirs::ScriptDirs;
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
                "{verb}: local file '{name}' not found beside init.scm"
            )));
        }
    }
    Ok(())
}

/// Whether `name` is in `declared_plugins` (case-insensitive).
fn is_declared(ctx: &SteelCtx, name: &str) -> bool {
    ctx.registries
        .declared_plugins
        .iter()
        .any(|d| d.eq_ignore_ascii_case(name))
}

/// PLUM compat: records `name` in `declared_plugins` if not already present,
/// regardless of whether the plugin resolves on disk. PLUM reads this list to
/// know what to install on `:plum-install-plugins`.
fn record_declared(ctx: &mut SteelCtx, name: &str) {
    if !is_declared(ctx, name) {
        ctx.registries.declared_plugins.push(name.to_string());
    }
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

/// Builds a quoted Steel string literal for `path` (`"…"`, backslashes
/// doubled so Windows paths like `C:\Users\…` survive embedding, since `\U` etc.
/// are invalid Steel escapes), or `None` if `path` contains `"`, which no
/// amount of escaping can embed in a Scheme string literal.
///
/// Also exposed as [`crate::steel_path_literal`] for test call sites (any
/// `.scm` source built by string interpolation from a real filesystem path:
/// `open-buffer!`, `picker!`, `require`, …) that would otherwise hand-roll
/// this same escaping per site.
pub fn steel_path_literal(path: &std::path::Path) -> Option<String> {
    let raw = path.to_string_lossy();
    if raw.contains('"') {
        return None;
    }
    let escaped = raw.replace('\\', "\\\\");
    Some(format!("\"{escaped}\""))
}

/// Builds `(require "<abs path>")`, rejecting a path containing `"` (which
/// can't be embedded in a Steel string literal) via [`steel_path_literal`].
///
/// `kind` names what `path` is, for the error message (`"plugin"` /
/// `"plugin manifest"`). `on_unquotable` runs (for its side effect only)
/// before the shared error is raised. `begin_lazy_activation` uses it to
/// mark the plugin `Failed` first; `load_plugin` has no plugin-stack state to
/// unwind yet, so passes a no-op.
fn require_program_for_path(
    path: &std::path::Path,
    kind: &str,
    on_unquotable: impl FnOnce(),
) -> Result<String, SteelErr> {
    let Some(literal) = steel_path_literal(path) else {
        on_unquotable();
        steel::stop!(Generic =>
            "{} path contains '\"', so it cannot be embedded in require: {}", kind, path.display());
    };
    Ok(format!("(require {literal})"))
}

/// Gate for plugin-registration verbs (`load-plugin!`, `declare-plugin!`).
///
/// Both verbs are valid only at the top level of `init.scm`, i.e. only
/// [`crate::context::EvalMode::Init`]. A plugin can never load or declare
/// another plugin: dependency declarations are the user's / plugin-manager's
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

/// Error label for a `declare-plugin!` keyword-argument decode.
///
/// Inside manifest resolution the offending code is the *plugin's*
/// `manifest.scm`, not the user's `init.scm` that the init-eval error prefix
/// will otherwise imply, so name it, and the plugin, explicitly.
fn declare_arg_label(ctx: &SteelCtx, keyword: &str) -> String {
    match &ctx.manifest_resolving {
        Some(id) => format!("declare-plugin! {keyword} in manifest.scm for '{id}'"),
        None => format!("declare-plugin! {keyword}"),
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
    ensure_top_level(ctx, "declare-plugin!")?;
    let plugin_id = PluginId::parse(&name).map_err(generic_err)?;
    let entry_file = entry_file_arg(entry)?;
    if plugin_id.is_local() && entry_file.is_some() {
        return Err(generic_err(format!(
            "declare-plugin!: '{name}' is a single file; #:entry applies to installed plugins"
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
            "declare-plugin!: manifest.scm for '{expected}' must declare '{expected}', not '{name}'"
        )));
    }

    // `plugin_configs` is written only by `load-plugin!`, so a key means the
    // plugin was already loaded, and by now its manifest or `plugin.scm` has
    // decided its entries.
    if ctx.manifest_resolving.is_none() && ctx.registries.plugin_configs.contains_key(&plugin_id) {
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
    // below: a malformed entry must leave `declared_plugins` untouched, or PLUM
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

    // Malformed name (not a collision) → hard error, same rule as
    // define-command!.  A name that can't survive quoting is a typo. This
    // check is independent of the plugin's on-disk path, so it runs
    // regardless of whether the plugin turns out to be absent.
    for cmd in cmd_list.iter().chain(typed_cmd_list.iter()) {
        if cmd.contains('"') || cmd.contains('\\') {
            steel::stop!(Generic =>
                "declare-plugin!: command name '{}' must not contain '\"' or '\\'", cmd);
        }
    }

    // Hard error: nothing declared at all. Checked against the raw (unfiltered)
    // lists, before path resolution: collision filtering only happens once the
    // plugin is confirmed present on disk (see below), so a non-empty
    // `cmd_list`/`typed_cmd_list` always skips this branch regardless of what
    // filtering later drops.
    if !has_trigger(&cmd_list, &typed_cmd_list, &evt_list, &lang_list) {
        return Err(generic_err(format!(
            "declare-plugin!: '{name}' declares no activation entries; it could never be activated. \
             Add #:commands/#:typed-commands/#:events/#:languages."
        )));
    }

    let path = entry_path(ctx.dirs, &entry_id).map_err(generic_err)?;

    // A secondary entry naming a file that is missing from a plugin that is
    // installed is a misconfigured plugin, not a "not installed yet" state.
    // Raised before anything is recorded so a failed declare leaves no trace.
    if path.is_none() && !entry_id.file.is_main() {
        let plugin_installed = plugin_dir_for_id(&plugin_id, ctx.dirs)
            .map(|dir| path_exists(&dir))
            .transpose()
            .map_err(generic_err)?
            .unwrap_or(false);
        if plugin_installed {
            return Err(generic_err(format!(
                "declare-plugin!: '{name}' has no entry file '{}'",
                entry_id.file.as_str()
            )));
        }
    }

    if !plugin_id.is_local() {
        record_declared(ctx, &name);
    }

    // When the plugin file is absent on disk, it can never be activated:
    // collision-checking (which claims the name in the editor's registry) would
    // be pointless and would leave the name claimed with no path to clean it up
    // via drop_activations_for's usual load/fail transition.  For user/ plugins,
    // log Info, since absent is expected before :plum-install-plugins.  For core: plugins,
    // absent means a typo or broken HUME_RUNTIME; PLUM never installs core:
    // plugins, so it can't catch the error.  `declared_plugins` is already
    // recorded above for PLUM.
    let Some(path) = path else {
        absent_plugin(ctx, &plugin_id, &name, "declare-plugin!")?;
        return Ok(SteelVal::Void);
    };

    // Filter colliding command names against the editor's live registry,
    // reached only now that the plugin is confirmed on disk. Each collision
    // logs a non-fatal Error (visible in :messages) and the name is dropped.
    // `register_lazy_command`/`register_lazy_typed_command` claims the name
    // in the same registry `define-command!`/`define-typed-command!` and
    // native commands live in, so this is the single check for "is this name
    // available" across builtin, activation, and command-table names.
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
            "declare-plugin!: '{name}' declares no activation entries; \
             all #:commands/#:typed-commands entries conflicted with existing commands. \
             Fix the collision."
        )));
    }

    // Pre-seed cmd_owners so (command-plugin "cmd") resolves correctly before
    // the plugin body is evaluated (before activation).  Only for accepted
    // names: a filtered-out collision must not gain attribution here.
    for cmd in cmd_list.iter().chain(typed_cmd_list.iter()) {
        ctx.registries
            .cmd_owners
            .insert(cmd.clone(), Owner::Plugin(entry_id.clone()));
    }

    ctx.registries
        .lazy_registry
        .declare(entry_id, path, evt_list, lang_list);

    Ok(SteelVal::Void)
}

/// The directory a plugin's files live in, given its id: `core:` plugins
/// under `runtime_dir`, `user/repo` plugins under `data_dir`, a local file's
/// own directory under `init_dir`. `None` when the relevant root is unset
/// (`HOME`/`APPDATA` unset for user plugins, no `init.scm` for a local file).
fn plugin_dir_for_id(plugin_id: &PluginId, dirs: &ScriptDirs) -> Option<std::path::PathBuf> {
    match plugin_id {
        PluginId::Core(core_name) => dirs
            .runtime_dir
            .as_deref()
            .map(|rt| rt.join("plugins").join("core").join(core_name)),
        // When data_dir is None (HOME/APPDATA unset), user plugins cannot be
        // resolved, so return None rather than panicking.
        PluginId::User { user, repo } => dirs
            .data_dir
            .as_deref()
            .map(|d| d.join("plugins").join(user).join(repo)),
        PluginId::Local(path) => dirs
            .init_dir
            .as_deref()
            .and_then(|d| d.join(path).parent().map(std::path::Path::to_path_buf)),
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

/// The file `entry` loads, if it exists on disk. A local plugin's one entry is
/// the file its id names; an installed plugin's entry is a file in its
/// directory. Errors when a local plugin is named with no `init.scm` to
/// resolve it against.
fn entry_path(dirs: &ScriptDirs, entry: &EntryId) -> Result<Option<std::path::PathBuf>, String> {
    let path = match &entry.plugin {
        PluginId::Local(rel) => match dirs.init_dir.as_deref() {
            Some(init_dir) => init_dir.join(rel),
            None => {
                return Err(format!(
                    "local plugin '{}' can only be declared from init.scm",
                    entry.plugin
                ));
            }
        },
        installed => match plugin_dir_for_id(installed, dirs) {
            Some(dir) => dir.join(entry.file.as_str()),
            None => return Ok(None),
        },
    };
    Ok(path_exists(&path)?.then_some(path))
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

/// `(%load-plugin! name config)`: Rust primitive backing the Scheme
/// `load-plugin!` wrapper.
///
/// Top-level only (`ensure_top_level`): a plugin can never load another
/// plugin, and a manifest being evaluated cannot call it.
///
/// Stores `config` unconditionally, replacing any earlier one, and records the
/// plugin for PLUM. A plugin that already has a declared entry (declared by
/// the user, loaded before, or evaluated through its manifest) is left as it
/// is. Otherwise the plugin's directory decides: a `manifest.scm` makes it
/// lazy and the returned `(require "<abs manifest.scm>")` string is evaluated
/// by the wrapper, which hands the outcome to `%finish-manifest-load!`; with
/// only a `plugin.scm` the main entry is declared and `#t` asks the wrapper to
/// activate it now. `#f` means nothing is left to do. A directory with neither
/// file is an error.
pub(crate) fn load_plugin(ctx: &mut SteelCtx, name: String, config: SteelVal) -> SteelResult {
    ensure_top_level(ctx, "load-plugin!")?;
    if ctx.manifest_resolving.is_some() {
        steel::stop!(Generic =>
            "load-plugin!: cannot be called from a manifest.scm; a manifest declares its own \
             plugin with declare-plugin!");
    }
    let plugin_id = PluginId::parse_installed(&name).map_err(generic_err)?;

    ctx.registries
        .plugin_configs
        .insert(plugin_id.clone(), config);
    let named_before = is_declared(ctx, &name);
    record_declared(ctx, &name);

    if ctx.registries.lazy_registry.declares_plugin(&plugin_id) {
        return Ok(SteelVal::BoolV(false));
    }
    let Some(dir) = plugin_dir_for_id(&plugin_id, ctx.dirs) else {
        return absent_load(ctx, &plugin_id, &name, named_before);
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
    if !path_exists(&dir).map_err(generic_err)? {
        return absent_load(ctx, &plugin_id, &name, named_before);
    }
    Err(generic_err(format!(
        "load-plugin!: '{name}' has neither manifest.scm nor plugin.scm in {}",
        dir.display()
    )))
}

/// `load-plugin!` for a plugin with no directory: reported once, unless an
/// earlier `declare-plugin!` already named it.
fn absent_load(
    ctx: &mut SteelCtx,
    plugin_id: &PluginId,
    name: &str,
    named_before: bool,
) -> SteelResult {
    if !named_before {
        absent_plugin(ctx, plugin_id, name, "load-plugin!")?;
    }
    Ok(SteelVal::BoolV(false))
}

/// Maximum nesting depth for concurrent inline plugin activations.
///
/// A depth of 16 is unreachable in practice (plugins rarely chain more than
/// 2–3 levels deep) but stops a misconfigured cycle that slipped past the
/// `Loading` guard from recursing until the stack overflows.
const MAX_ACTIVATION_DEPTH: usize = 16;

/// Marks `id` `Failed` and runs the same cleanup `finish_lazy_activation`
/// would on failure: `drop_activations_for` (expired activation-event/language
/// entries) and `unregister_lazy_stubs_of` (dead `Lazy` command stub).
///
/// Shared by `begin_lazy_activation`'s two pre-body-eval raise paths (depth
/// limit, unquotable path) and `finish_lazy_activation`'s failure branch.
/// Both leave a `Failed` plugin with no live activation footprint. Command/
/// hook rollback stays out of this helper: `begin_lazy_activation` raises
/// before the body ever runs, so no commands or hooks are registered under
/// this id yet.
fn fail_plugin_activation(ctx: &mut SteelCtx, id: &EntryId) {
    ctx.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Failed);
    ctx.registries.lazy_registry.drop_activations_for(id);
    ctx.host.commands().unregister_lazy_stubs_of(id);
}

/// Decodes an entry-file argument: `#f` is `None` (the main entry by
/// default), a string names that file.
fn entry_file_arg(entry: SteelVal) -> Result<Option<EntryFile>, SteelErr> {
    optional_string_arg(entry, "plugin entry")?
        .map(|file| EntryFile::parse(&file).map_err(generic_err))
        .transpose()
}

/// Parses the `(plugin entry-file)` pair every inline-activation primitive
/// takes into the entry it names.
fn entry_id_from_args(plugin: &str, entry: SteelVal) -> Result<EntryId, SteelErr> {
    let plugin = PluginId::parse(plugin).map_err(generic_err)?;
    Ok(EntryId::new(
        plugin,
        entry_file_arg(entry)?.unwrap_or_else(EntryFile::main),
    ))
}

/// `(%begin-lazy-activation! plugin entry)`: Rust primitive for inline activation.
///
/// Called from the BOOTSTRAP `%activate-plugin-inline!` helper immediately before
/// `(hm.eval-string require-string)`.  If the plugin is `Declared`, transitions
/// to `Loading`, pushes `plugin_stack`, and returns the `(require "<abs>")` string.
/// Returns `#f` for the cycle/idempotency guard (Loading/Loaded/Failed/absent) so
/// `%activate-plugin-inline!` becomes a no-op without error.
pub(crate) fn begin_lazy_activation(
    ctx: &mut SteelCtx,
    plugin: String,
    entry: SteelVal,
) -> SteelResult {
    let id = entry_id_from_args(&plugin, entry)?;

    let path = match ctx.registries.lazy_registry.plugins.get(&id) {
        Some(PluginState::Declared { path }) => path.clone(),
        Some(PluginState::Loading | PluginState::Loaded | PluginState::Failed) | None => {
            return Ok(SteelVal::BoolV(false));
        }
    };

    if ctx.plugin_stack.len() >= MAX_ACTIVATION_DEPTH {
        fail_plugin_activation(ctx, &id);
        steel::stop!(Generic =>
            "%begin-lazy-activation!: activation depth limit ({}) exceeded \
             (check for circular load-plugin! chains); '{}' marked Failed",
            MAX_ACTIVATION_DEPTH, id);
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

/// `(%finish-lazy-activation! plugin entry error)`: called by
/// `%activate-plugin-inline!` after the plugin body's `eval-string`. `error` is
/// `#f` on success or the caught exception value. Pops `plugin_stack` and
/// moves the plugin to `Loaded` or `Failed`.
///
/// A failure is recorded in `ctx.failed_activations` rather than returned, so
/// a failing plugin never aborts the enclosing eval; `run_steel_session`
/// reports it when the session ends.
///
/// On failure the body's footprint is rolled back: its commands leave
/// `command_table`/`typed_command_table`, `cmd_owners` and the
/// `CommandRegistry`; its hooks are removed by owner; and
/// `ctx.pop_effect_marks` drops its queued effects (key binds, LSP servers,
/// languages), except those already committed by a nested activation.
/// Hooks are removed by owner identity instead of queued as effects because
/// `register-hook!` must mutate the registry immediately: during startup
/// `Editor::init_scripting` still holds the host in a local, so a queued hook
/// effect would be dropped.
pub(crate) fn finish_lazy_activation(
    ctx: &mut SteelCtx,
    plugin: String,
    entry: SteelVal,
    error: SteelVal,
) -> SteelResult {
    // `plugin_stack.pop()` and `pop_effect_marks` below must run
    // unconditionally, before any fallible decode. `begin_lazy_activation`
    // pushed both unconditionally, and this is their only pairing site. A
    // `?` short-circuit on `error`'s decode here would leave them
    // permanently unbalanced whenever `error` fails to decode as `#f` or a
    // caught error value, skewing `EvalMode` for the rest of the session
    // and leaking the queued effects a never-popped mark hides. A decode
    // failure becomes the failure *reason* instead: `unwrap_or_else` folds
    // it into `Some`, same shape as a body's own raised error.
    ctx.plugin_stack.pop();
    let error = optional_steel_error_arg(error, "%finish-lazy-activation!").unwrap_or_else(Some);
    ctx.pop_effect_marks(error.is_none());

    let id = entry_id_from_args(&plugin, entry)?;

    if let Some(err) = error {
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
            // Only one of the two ever has an entry for a given name (the
            // other remove is a no-op), since command_table/typed_command_table
            // are disjoint by kind (see typed_command_table's own doc).
            ctx.registries.command_table.remove(&name);
            ctx.registries.typed_command_table.remove(&name);
            ctx.registries.cmd_owners.remove(&name);
            ctx.host.commands().unregister_command(&name);
        }

        // Roll back hooks the failed body registered.
        ctx.registries.hooks.remove_owned_by(&id);

        ctx.failed_activations.push((id, err));
    } else {
        ctx.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loaded);
        ctx.registries.lazy_registry.drop_activations_for(&id);
        // Drop any `Lazy` stub the plugin didn't replace via `define-command!`:
        // dead weight now that the plugin is Loaded and won't re-run its body.
        ctx.host.commands().unregister_lazy_stubs_of(&id);
    }

    Ok(SteelVal::Void)
}

/// `(%lazy-command-owner name)`: return the owning entry as a `(plugin
/// entry-file)` list if `name` is a registered *mappable* activation command,
/// or `#f` if not.  Used by
/// `%dispatch-command!` to decide whether a `command_table` miss should trigger
/// inline activation.
///
/// Mappable-only: `%dispatch-command!` backs `call!`, which can
/// never reach a typed command (`typed_command_table` is a separate table;
/// see its own doc). Reporting a typed-only stub as activatable here would
/// load the plugin for a lookup that misses again right after and errors:
/// a permanent side effect for a call that could never succeed.
pub(crate) fn lazy_command_owner(ctx: &mut SteelCtx, name: String) -> SteelResult {
    match ctx.host.commands().lazy_mappable_command_owner(&name) {
        Some(id) => vec![
            id.plugin.to_string(),
            id.file.as_str().to_string(),
            id.to_string(),
        ]
        .into_steelval()
        .map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(%entry-loaded? plugin entry)`: `#t` when that entry's body has been
/// evaluated successfully.
pub(crate) fn entry_loaded(ctx: &mut SteelCtx, plugin: String, entry: SteelVal) -> SteelResult {
    let id = entry_id_from_args(&plugin, entry)?;
    Ok(SteelVal::BoolV(matches!(
        ctx.registries.lazy_registry.plugins.get(&id),
        Some(PluginState::Loaded)
    )))
}

/// `(%finish-manifest-load! name error)`: Rust primitive; the tail half
/// of `load-plugin!`'s manifest path (mirrors `%finish-lazy-activation!`,
/// including its `error` argument convention and its unconditional-before-
/// any-fallible-decode ordering; see that function's doc for why).
///
/// Clears `manifest_resolving` and pops the effect mark `load_plugin`
/// pushed unconditionally, before decoding `error`. On success, verifies the
/// manifest actually declared the plugin: a `manifest.scm` that evaluates
/// without error but never calls `declare-plugin!` would otherwise leave the
/// plugin silently undeclared; that check's own failure is raised, caught by
/// the same `with-handler` in `bootstrap.scm`, and reaches this function a
/// second time as a genuine failure. On failure, rolls the plugin back to
/// `Failed` via `fail_plugin_activation` (same helper a lazy activation
/// failure uses), undoing the declarations manifest.scm committed before a
/// later top-level form in the same file raised (see the `Some(err)` arm
/// below), then records the failure the way a body error is into
/// `ctx.failed_activations` (see `run_steel_session`).
pub(crate) fn finish_manifest_load(
    ctx: &mut SteelCtx,
    name: String,
    error: SteelVal,
) -> SteelResult {
    // `manifest_resolving` must clear unconditionally, before any fallible
    // decode, mirroring `finish_lazy_activation`'s stack/marks discipline.
    // Otherwise a decode failure would leave manifest resolution permanently
    // "in progress", and every later `load-plugin!` would hard-error on the
    // reentrancy guard.
    ctx.manifest_resolving = None;
    let error = optional_steel_error_arg(error, "%finish-manifest-load!").unwrap_or_else(Some);
    ctx.pop_effect_marks(error.is_none());

    let id = PluginId::parse_installed(&name).map_err(generic_err)?;

    match error {
        None if !ctx.registries.lazy_registry.declares_plugin(&id) => {
            return Err(generic_err(format!(
                "load-plugin!: manifest.scm for '{name}' did not declare '{name}': a \
                 manifest.scm must call (declare-plugin! \"{name}\" …) with at least one \
                 activation entry"
            )));
        }
        None => {}
        Some(err) => {
            // A manifest declares its entries with `declare-plugin!` calls,
            // some of which may commit before a *later* top-level form in the
            // same file raises. `hm.eval-string` runs manifest.scm as one program, so
            // that self-declare's `Declared` state is already committed, and
            // rolling the whole manifest resolution back to `Failed` (the
            // same helper a lazy activation failure uses) undoes it, so a
            // half-evaluated manifest.scm never leaves a live command stub
            // behind for a plugin the user was just told failed to load.
            // `declared_plugins` (the flat PLUM-visible list) is untouched
            // (`fail_plugin_activation` never touches it), so PLUM still
            // offers to install/update the plugin.
            let main = EntryId::main(id.clone());
            let declared = ctx.registries.lazy_registry.entries_of(&id);
            for entry in declared.iter().chain(std::iter::once(&main)) {
                fail_plugin_activation(ctx, entry);
            }
            ctx.failed_activations.push((main, err));
        }
    }

    Ok(SteelVal::Void)
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
        .declared_plugins
        .iter()
        .map(|s| SteelVal::StringV(s.as_str().into()))
        .collect();
    vals.into_steelval().map_err(generic_err)
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
    match ctx.registries.plugin_configs.get(&id.plugin) {
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

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// Parsing tests (valid/invalid plugin names, segments) live in
// `hume_scripting::attribution::tests` alongside `PluginId::parse`.  The tests here
// cover only the builtins' Steel-facing behaviour.

#[cfg(test)]
mod tests;
