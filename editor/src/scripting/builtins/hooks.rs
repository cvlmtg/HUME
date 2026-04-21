//! `(register-hook! 'hook-name proc)` builtin.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::scripting::SteelCtx;
use crate::scripting::hooks::HookId;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(register-hook! 'name proc)` — register `proc` as a handler for the
/// named hook.  Must be called during init / plugin load (`is_init = true`).
///
/// `name` must be a symbol matching one of the known hook names:
/// `on-buffer-open`, `on-buffer-close`, `on-buffer-save`, `on-edit`,
/// `on-mode-change`.
pub(crate) fn register_hook(ctx: &mut SteelCtx, name: SteelVal, proc: SteelVal) -> SteelResult {
    if !ctx.is_init {
        steel::stop!(Generic => "register-hook!: can only be called during init/plugin load");
    }
    let name_str = match &name {
        SteelVal::SymbolV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch => "register-hook!: expected a symbol, got {:?}", name),
    };
    let hook_id = match HookId::from_symbol(&name_str) {
        Some(id) => id,
        None => steel::stop!(
            Generic =>
            "register-hook!: unknown hook '{}'; known hooks: {}",
            name_str,
            HookId::all_names().collect::<Vec<_>>().join(", ")
        ),
    };
    let owner = ctx.plugin_stack.current_owner();
    ctx.hooks.register(hook_id, owner, proc);
    Ok(SteelVal::Void)
}
