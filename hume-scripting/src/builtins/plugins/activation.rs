//! Plugin activation: the builtins `bootstrap.scm` brackets every plugin
//! body with, and the state each moves its entry through.
//!
//! ## Activation state machine
//!
//! ```text
//! Declared ──── %activate-plugin-inline! ────► Loading ──┬──► Loaded
//!                                                        └──► Failed
//!
//! Loaded / Failed / Loading / absent  ──► no-op (#f guard in %begin-lazy-activation!)
//! ```
//!
//! All plugin activation is synchronous/inline:
//! - `load-plugin!` (init.scm): the BOOTSTRAP Scheme wrapper calls `%load-plugin!`,
//!   which either returns a manifest to evaluate (lazy) or asks for
//!   `%activate-plugin-inline!` (inline body eval via `hm.eval-string`).
//! - Lazy keypress dispatch: `%dispatch-command!` activates the owner inline when
//!   no mappable proc is defined yet, then retries.
//! - Event/language activations: `activate_plugin_inline` (Rust) bounces into
//!   `(%activate-plugin-inline! id)` by a direct function call (not source),
//!   using the `ScriptingHost`'s one persistent watchdog.

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use super::entry_file_arg;
use super::paths::require_program_for_path;
use crate::SteelCtx;
use crate::attribution::{EntryFile, EntryId, PluginId};
use crate::builtins::SteelResult;
use crate::builtins::args::optional_steel_error_arg;
use crate::builtins::errors::generic_err;
use crate::lazy::PluginState;
use crate::plugin_record::Resolution;

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

/// Parses the `(plugin entry-file)` pair `%begin-lazy-activation!` and
/// `%entry-loaded?` take into the entry it names.
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
/// to `Loading`, pushes its body onto `attribution`, and returns the `(require "<abs>")` string.
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

    ctx.host
        .permit_plugin_activation(&id)
        .map_err(generic_err)?;

    if ctx.attribution.body_depth() >= MAX_ACTIVATION_DEPTH {
        fail_plugin_activation(ctx, &id);
        steel::stop!(Generic =>
            "activation depth limit ({}) exceeded \
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
    ctx.attribution.push_body(id);
    ctx.mark_effects();

    Ok(SteelVal::StringV(require_program.into()))
}

/// `(%finish-lazy-activation! error)`: called by `%activate-plugin-inline!`
/// after the plugin body's `eval-string`. `error` is `#f` on success or the
/// caught exception value. Pops the entry `%begin-lazy-activation!` pushed
/// onto `attribution` and moves it to `Loaded` or `Failed`. Raises when no
/// activation is in progress.
///
/// A failure is recorded in `ctx.failed_activations` rather than returned, so
/// a failing plugin never aborts the enclosing eval; `run_steel_session`
/// reports it when the session ends.
///
/// On failure the body's footprint is rolled back: the commands and hooks
/// registered while it ran (whoever wrote the code registering them) leave
/// their registries; and `ctx.pop_effect_marks` drops its queued effects
/// (key binds, LSP servers, languages), except those already committed by a
/// nested activation. Hooks are removed by the body they were registered
/// in instead of queued as effects because
/// `register-hook!` must mutate the registry immediately: during startup
/// `Editor::init_scripting` still holds the host in a local, so a queued hook
/// effect would be dropped.
pub(crate) fn finish_lazy_activation(ctx: &mut SteelCtx, error: SteelVal) -> SteelResult {
    // `begin_lazy_activation` pushes the entry and an effect mark together,
    // and this is their only pairing site. An empty stack means no mark of
    // this activation exists either, so it raises before touching the marks.
    // Past that point nothing may short-circuit: an unpopped mark would hide
    // queued effects and leave the entry `Loading`. An `error` that decodes
    // as neither `#f` nor a caught error value becomes the failure reason,
    // same shape as a body's own raised error.
    let Some(id) = ctx.attribution.pop_body() else {
        steel::stop!(Generic => "no plugin activation in progress");
    };
    let error = optional_steel_error_arg(error, "%finish-lazy-activation!").unwrap_or_else(Some);
    ctx.pop_effect_marks(error.is_none());

    if let Some(err) = error {
        fail_activation_body(ctx, id, err);
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

/// Rolls a failed body's footprint back and records the failure for
/// `run_steel_session` to report: the entry turns `Failed`, and the commands
/// and hooks it registered are removed.
fn fail_activation_body(ctx: &mut SteelCtx, id: EntryId, err: SteelErr) {
    fail_plugin_activation(ctx, &id);
    ctx.host.commands().unregister_commands_registered_in(&id);
    ctx.registries.hooks.remove_registered_in(&id);
    ctx.failed_activations.push((id, err));
}

/// Fails `bodies`, innermost first, with `reason`: the state a caught error
/// in each leaves through `%finish-lazy-activation!`.
pub(crate) fn fail_open_bodies(ctx: &mut SteelCtx, bodies: Vec<EntryId>, reason: &SteelErr) {
    for id in bodies {
        ctx.pop_effect_marks(false);
        fail_activation_body(ctx, id, reason.clone());
    }
}

/// Fails every activation and manifest load still open above `depth` when a
/// session ends, innermost first, with `reason`: the state a caught error
/// leaves through `%finish-lazy-activation!`/`%finish-manifest-load!`. Their
/// handlers are Scheme, which a session stopped by its budget never runs;
/// the same holds for a `call!` frame's restore, which is dropped here too.
pub(crate) fn abandon_open_activations(ctx: &mut SteelCtx, depth: usize, reason: &SteelErr) {
    let bodies = ctx.attribution.drain_above(depth);
    fail_open_bodies(ctx, bodies, reason);
    if let Some(id) = ctx.manifest_resolving.take() {
        ctx.pop_effect_marks(false);
        if let Err(e) = fail_manifest_load(ctx, id, reason.clone()) {
            ctx.log(hume_core::Severity::Error, e.to_string());
        }
    }
}

/// `(%lazy-command-owner name)`: return the owning entry as a `(plugin
/// entry-file)` list if `name` is a registered *mappable* activation command,
/// or `#f` if not. Used by `%dispatch-command!` to decide whether a
/// mappable-proc miss should trigger inline activation.
///
/// Mappable-only: `%dispatch-command!` backs `call!`, which resolves a name
/// inside the VM, where only mappable commands are reachable. Reporting a
/// typed-only stub as activatable here would load the plugin for a lookup
/// that misses again right after and errors:
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
/// pushed unconditionally, before decoding `error`. A `manifest.scm` that
/// evaluates without error but never declares its plugin fails with its own
/// error, so the mark's effects are committed only for a manifest that
/// evaluated cleanly and declared the plugin.
///
/// On failure, marks the plugin's manifest failed and rolls its entries back
/// to `Failed` via `fail_plugin_activation` (the helper a lazy activation
/// failure uses), then records the failure the way a body error is into
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
    let resolving = ctx.manifest_resolving.take();
    let error = optional_steel_error_arg(error, "%finish-manifest-load!").unwrap_or_else(Some);
    let declared = resolving
        .as_ref()
        .is_some_and(|id| ctx.registries.lazy_registry.declares_plugin(id));
    let error = error.or_else(|| {
        (!declared).then(|| {
            generic_err(format!(
                "manifest.scm for '{name}' did not declare '{name}': a \
                 manifest.scm must call (declare-plugin! \"{name}\" …) with at least one \
                 activation entry"
            ))
        })
    });
    ctx.pop_effect_marks(error.is_none());
    let id = resolving
        .ok_or_else(|| generic_err(format!("no manifest is being resolved for '{name}'")))?;

    if let Some(err) = error {
        fail_manifest_load(ctx, id, err)?;
    }

    Ok(SteelVal::Void)
}

/// Marks `id`'s manifest failed and rolls its entries back to `Failed`, then
/// records the failure for `run_steel_session` to report.
///
/// A manifest declares its entries with `declare-plugin!` calls, some of
/// which may commit before a *later* top-level form in the same file raises.
/// `hm.eval-string` runs manifest.scm as one program, so rolling its entries
/// back to `Failed` undoes them: a half-evaluated manifest.scm never leaves a
/// live command stub behind for a plugin the user was just told failed to
/// load. The plugin stays in the PLUM list, so PLUM still offers to
/// install/update it.
fn fail_manifest_load(ctx: &mut SteelCtx, id: PluginId, err: SteelErr) -> Result<(), SteelErr> {
    ctx.registries
        .plugin_records
        .resolve(&id, Resolution::ManifestFailed)
        .map_err(generic_err)?;
    for entry in ctx.registries.lazy_registry.entries_of(&id) {
        fail_plugin_activation(ctx, &entry);
    }
    ctx.failed_activations.push((EntryId::main(id), err));
    Ok(())
}

#[cfg(test)]
mod tests;
