//! Server registration and per-language server lists, and the commands
//! that stop, restart and report a running server.

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{FromSteelVal, SteelVal};

use crate::types::{
    Effect, FeatureFilter, ListEntry, ListLayer, LspFeature, LspFeatureSet, LspServerTarget,
    PaneHandle, PendingLspServerOp, ServerName,
};
use crate::{PendingLspServerReg, SteelCtx};

use crate::builtins::SteelResult;
use crate::builtins::args::{
    BuiltinArg, ENV_PAIR_SHAPE, LivePane, hash_entry, list_of, list_to_string_pairs,
    list_to_strings, optional_json_arg, optional_list_items, string_arg, symbol_enum_arg,
    symbol_hash,
};
use crate::builtins::errors::{generic_err, require_cap};

/// A registration or list-entry server name, as [`ServerName::parse`]
/// accepts it.
pub(super) fn server_name_arg(val: SteelVal, ctx_name: &str) -> Result<ServerName, SteelErr> {
    let name = string_arg(val, ctx_name)?;
    ServerName::parse(&name).map_err(|e| generic_err(format!("{ctx_name}: {e}")))
}

/// `(%register-lsp-server! name command args init-options settings env)`
///
/// Callable from init.scm, plugin activation, or a command/hook body,
/// unlike `%define-language!`, this is not gated to init/activation-only.
/// Queues a last-wins registration keyed by `name`: applied at the end of
/// the *current* eval (see `Editor::apply_lsp_server_ops`), replacing any
/// existing registration of `name` and attaching already-open matching
/// buffers. `lsp-server-registered?` reads through the effect log, so it
/// reports this registration as live immediately, within the same eval.
///
/// `args` is a list of strings; `env` is a list of
/// `("KEY" . "VALUE")` dotted pairs, applied additively to the spawned
/// process's inherited environment. Pushes an
/// `Effect::LspServerOp(PendingLspServerOp::Register)`.
// Each param is a positional/keyword arg the `builtins!` table maps 1:1 from
// `register-lsp-server!`'s own Steel signature. Bundling them into a struct
// would break that direct correspondence for no benefit, since every arg is
// already decoded and validated independently right below.
#[allow(clippy::too_many_arguments)]
pub(crate) fn register_lsp_server(
    ctx: &mut SteelCtx,
    name: SteelVal,
    command: SteelVal,
    args_val: SteelVal,
    init_options: SteelVal,
    settings: SteelVal,
    env_val: SteelVal,
) -> SteelResult {
    let name = server_name_arg(name, "name")?;
    let command = string_arg(command, "command")?;
    let args = list_to_strings(args_val, "args")?;
    let init_options = optional_json_arg(init_options, "init-options")?;
    let settings = optional_json_arg(settings, "settings")?;
    let env = list_to_string_pairs(env_val, "env", ENV_PAIR_SHAPE)?;

    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Register(
        PendingLspServerReg {
            name,
            command,
            args,
            init_options,
            settings,
            env,
        },
    )));
    Ok(SteelVal::Void)
}

/// `(unregister-lsp-server! name)`: queues removal of `name`'s
/// registration, applied at the end of the current eval (see
/// `Editor::apply_lsp_server_ops`). Every buffer attached to an instance of
/// `name` detaches, and an instance no buffer uses stops. The stop happens
/// where the op falls among the eval's other server ops, so a
/// `register-lsp-server!` of the same name after it, in the same eval, spawns
/// a fresh server.
///
/// Idempotent: unregistering a name with no registration and/or no
/// running instance is not an error: `:lsp-uninstall` of an already-removed
/// or never-installed server must succeed silently.
///
/// Applies at end-of-eval, so within the *same* eval the server process is
/// still alive. A caller that must touch the on-disk server files after
/// shutdown (e.g. reinstalling on Windows, where a running binary's file is
/// locked) should do that work in a follow-up queued eval (e.g. via
/// `(after! 0 …)`), which runs strictly after this eval's drain reaps the
/// process.
pub(crate) fn unregister_lsp_server(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = server_name_arg(name, "name")?;
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Unregister { name }));
    Ok(SteelVal::Void)
}

/// One server-list entry: a server name, or a hash with a `'name` and at
/// most one of `'only-features`/`'except-features`. Every shape error
/// raises, so a typo cannot widen a filter.
fn list_entry_arg(item: SteelVal, ctx_name: &str) -> Result<ListEntry, SteelErr> {
    if !matches!(item, SteelVal::HashMapV(_)) {
        return Ok(ListEntry {
            name: server_name_arg(item, ctx_name)?,
            filter: FeatureFilter::All,
        });
    }
    let entry = hash_entry(
        item,
        ctx_name,
        &["name", "only-features", "except-features"],
    )?;
    let name = server_name_arg(entry.required("name")?, ctx_name)?;
    let filter = match (
        entry.optional("only-features"),
        entry.optional("except-features"),
    ) {
        (None, None) => FeatureFilter::All,
        (Some(only), None) => {
            FeatureFilter::Only(feature_set_arg(only, "only-features", ctx_name)?)
        }
        (None, Some(except)) => {
            FeatureFilter::Except(feature_set_arg(except, "except-features", ctx_name)?)
        }
        (Some(_), Some(_)) => {
            return Err(generic_err(format!(
                "{ctx_name}: '{name}' gives both 'only-features and 'except-features"
            )));
        }
    };
    Ok(ListEntry { name, filter })
}

/// A list of feature names, the value of a list entry's `key`.
fn feature_set_arg(val: SteelVal, key: &str, ctx_name: &str) -> Result<LspFeatureSet, SteelErr> {
    let label = format!("{ctx_name} '{key}");
    list_to_strings(val, &label)?
        .iter()
        .map(|name| symbol_enum_arg(name, &label, &LspFeature::NAMED))
        .collect()
}

/// The one implementation behind `set-language-servers!` and
/// `set-default-language-servers!`: they differ only in `layer`. `entries`
/// is a list of server names/hashes, or `#f` to clear the layer.
fn queue_language_servers(
    ctx: &mut SteelCtx,
    language: SteelVal,
    entries: SteelVal,
    layer: ListLayer,
) -> SteelResult {
    let language = string_arg(language, "language")?;
    let entries = optional_list_items(entries, "entries")?
        .map(|items| {
            let mut decoded: Vec<ListEntry> = Vec::with_capacity(items.len());
            for item in items {
                let entry = list_entry_arg(item, "entries")?;
                if decoded.iter().any(|e| e.name == entry.name) {
                    return Err(generic_err(format!(
                        "entries: '{}' is listed twice",
                        entry.name
                    )));
                }
                decoded.push(entry);
            }
            Ok(decoded)
        })
        .transpose()?;
    ctx.push_effect(Effect::LspServerOp(
        PendingLspServerOp::SetLanguageServers {
            language,
            layer,
            entries,
        },
    ));
    Ok(SteelVal::Void)
}

/// `(set-language-servers! language entries)`: the user's own ordered server
/// list for `language`, which beats any default list whatever the call order.
pub(crate) fn set_language_servers(
    ctx: &mut SteelCtx,
    language: SteelVal,
    entries: SteelVal,
) -> SteelResult {
    queue_language_servers(ctx, language, entries, ListLayer::User)
}

/// `(set-default-language-servers! language entries)`: the list a plugin
/// ships for `language`, used only while the user has set none.
pub(crate) fn set_default_language_servers(
    ctx: &mut SteelCtx,
    language: SteelVal,
    entries: SteelVal,
) -> SteelResult {
    queue_language_servers(ctx, language, entries, ListLayer::Default)
}

/// `(lsp-language-servers language)` → the servers `language`'s buffers
/// attach to, in order: one `(hash 'name n)` per server, with
/// `'only-features` or `'except-features` (a list of feature symbols) when
/// its list entry filters it.
pub(crate) fn lsp_language_servers(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = string_arg(language, "language")?;
    let entries = require_cap(ctx.host.lsp())?.lsp_language_servers(&language);
    let features =
        |set: LspFeatureSet| list_of(set.iter().map(|f| SteelVal::SymbolV(f.name().into())));
    Ok(list_of(entries.into_iter().map(|entry| {
        let name = ("name", SteelVal::StringV(entry.name.as_str().into()));
        match entry.filter {
            FeatureFilter::All => symbol_hash([name]),
            FeatureFilter::Only(set) => symbol_hash([name, ("only-features", features(set))]),
            FeatureFilter::Except(set) => symbol_hash([name, ("except-features", features(set))]),
        }
    })))
}

/// `(lsp-stop! target)`: `target` a pane (every server attached to its
/// buffer) or a string/symbol (every running instance of that server name).
/// Queues a stop, applied at the end of the current eval (see
/// `Editor::apply_lsp_server_ops`); the report of how many servers stopped is
/// emitted by that same drain.
pub(crate) fn lsp_stop(ctx: &mut SteelCtx, target: LspServerTarget) -> SteelResult {
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Stop { target }));
    Ok(SteelVal::Void)
}

/// `(lsp-restart! target)`: same argument shape as `lsp-stop!`. Queues a
/// stop-then-respawn, applied at the end of the current eval.
pub(crate) fn lsp_restart(ctx: &mut SteelCtx, target: LspServerTarget) -> SteelResult {
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Restart { target }));
    Ok(SteelVal::Void)
}

/// `(lsp-show-status! pane)`: queues opening the `[lsp-status]` read-only
/// view, applied at the end of the current eval. Kind-A: `pane` must be the
/// focused pane, since the view opens anchored there.
pub(crate) fn lsp_show_status(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    ctx.host
        .buffers()
        .require_focused_pane(pane)
        .map_err(generic_err)?;
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::ShowStatus));
    Ok(SteelVal::Void)
}

/// `(lsp-stop! target)` / `(lsp-restart! target)`'s `target` argument, ahead
/// of [`BuiltinArg::resolve`]'s liveness check on the `Buffer` case. A pane
/// decodes like [`LivePane`] (only its buffer is used: this is a kind-C,
/// buffer-only operation), a string or symbol names a registered server
/// (required: there is no "focused buffer" fallback to decode `#f` into).
#[derive(Debug)]
pub(crate) enum LspTargetArg {
    Buffer(LivePane),
    Name(String),
}

impl FromSteelVal for LspTargetArg {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        if let Ok(pane) = LivePane::from_steelval(val) {
            return Ok(LspTargetArg::Buffer(pane));
        }
        match val {
            SteelVal::StringV(s) => Ok(LspTargetArg::Name(s.to_string())),
            SteelVal::SymbolV(s) => Ok(LspTargetArg::Name(s.to_string())),
            _ => Err(SteelErr::new(
                ErrorKind::TypeMismatch,
                "expected a pane or a server name".to_string(),
            )),
        }
    }
}

impl BuiltinArg for LspTargetArg {
    type Out = crate::types::LspServerTarget;
    fn resolve(
        self,
        ctx: &mut crate::SteelCtx,
        arg: &'static str,
    ) -> Result<crate::types::LspServerTarget, SteelErr> {
        match self {
            LspTargetArg::Buffer(pane) => Ok(crate::types::LspServerTarget::Buffer(
                pane.resolve(ctx, arg)?.buffer(),
            )),
            LspTargetArg::Name(server) => crate::types::ServerName::parse(&server)
                .map(crate::types::LspServerTarget::Name)
                .map_err(|e| generic_err(format!("{arg}: {e}"))),
        }
    }
}
