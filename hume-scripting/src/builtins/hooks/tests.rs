use super::*;
use crate::test_support::SteelCtxTestHarness;

/// `register-hook!` is blocked in plain command mode (init/plugin-load only).
///
/// Fail oracle: change `register-hook!`'s table entry from `config` to
/// `open` → the hook is silently registered from a command body, allowing
/// plugins to change global behaviour at runtime.
#[test]
fn register_hook_blocked_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_config(&h.ctx(), "register-hook!"); // EvalMode::Command
    assert!(result.is_err(), "register-hook! must error in command mode");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("init"),
        "error must mention 'init'; got: {msg}"
    );
}

/// `register-hook!` errors when the first argument is not a symbol.
///
/// Fail oracle: remove the symbol check → strings or integers would be silently
/// accepted, and the hook name lookup would silently fail.
#[test]
fn register_hook_non_symbol_arg_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = register_hook(
        &mut ctx,
        SteelVal::StringV("on-buffer-open".into()), // should be a symbol
        SteelVal::BoolV(true),
    );
    assert!(result.is_err(), "register-hook! must reject a string name");
}

/// `register-hook!` errors for an unknown hook name.
///
/// Fail oracle: remove the `known_event_names()` lookup guard → typos
/// silently register a hook that is never fired.
#[test]
fn register_hook_unknown_hook_name_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = register_hook(
        &mut ctx,
        SteelVal::SymbolV("on-nonexistent-event".into()),
        SteelVal::BoolV(true),
    );
    assert!(
        result.is_err(),
        "register-hook! must reject unknown hook names"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("unknown hook"),
        "error must mention 'unknown hook'; got: {msg}"
    );
    assert!(
        msg.contains("on-buffer-open"),
        "error must list the valid names (from NULL_HOST_EVENT_NAMES); got: {msg}"
    );
}

/// `register-hook!` in init mode with a valid name registers the handler.
///
/// Fail oracle: make `register` a no-op → `handlers_for` returns empty slice →
/// last assert fires.
#[test]
fn register_hook_valid_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = register_hook(
            &mut ctx,
            SteelVal::SymbolV("on-buffer-save".into()),
            SteelVal::BoolV(true), // dummy proc — registry just stores SteelVal
        );
        assert!(result.is_ok(), "register-hook! must succeed in init mode");
    }
    assert_eq!(
        h.registries.hooks.handlers_for("on-buffer-save").len(),
        1,
        "one handler must be registered for on-buffer-save"
    );
    assert!(
        h.registries.hooks.handlers_for("on-buffer-save")[0]
            .owner
            .is_none(),
        "a top-level (non-plugin) registration must have no owner"
    );
}

/// `register-hook!` is also valid during plugin activation (plugin_stack non-empty).
#[test]
fn register_hook_valid_during_plugin_load() {
    use crate::attribution::PluginId;
    let mut h = SteelCtxTestHarness::new();
    // Simulate being inside a plugin body.
    h.plugin_stack
        .push(PluginId::parse("core:myplugin").unwrap());
    {
        let mut ctx = h.ctx(); // EvalMode::PluginActivation → allowed
        let result = register_hook(
            &mut ctx,
            SteelVal::SymbolV("on-buffer-open".into()),
            SteelVal::IntV(42),
        );
        assert!(
            result.is_ok(),
            "register-hook! must succeed during plugin load"
        );
    }
    assert!(!h.registries.hooks.is_empty_for("on-buffer-open"));
    assert_eq!(
        h.registries.hooks.handlers_for("on-buffer-open")[0].owner,
        Some(PluginId::parse("core:myplugin").unwrap()),
        "a plugin-body registration must be attributed to the currently-executing plugin"
    );
}

/// Validation is genuinely host-driven, not a compiled-in table:
/// `NullHost`'s known-names fixture deliberately diverges from the editor's
/// real event set (see `NULL_HOST_EVENT_NAMES`'s doc comment) — it includes
/// a synthetic `on-stub-only` name the editor never defines, and omits real
/// editor events like `on-lsp-attach`. `register-hook!` must follow the
/// host's list exactly in both directions.
///
/// Fail oracle: if `register_hook` consulted a compiled-in name list instead
/// of `ctx.host.events().known_event_names()`, `on-stub-only` would be
/// rejected (it's not a real `EditorEvent`) and `on-lsp-attach` would be
/// accepted (it is) — both assertions below would flip.
#[test]
fn register_hook_validates_against_the_host_not_a_compiled_in_table() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();

    let stub_only = register_hook(
        &mut ctx,
        SteelVal::SymbolV("on-stub-only".into()),
        SteelVal::BoolV(true),
    );
    assert!(
        stub_only.is_ok(),
        "a name absent from the editor but present on the host must be accepted"
    );

    let real_editor_event = register_hook(
        &mut ctx,
        SteelVal::SymbolV("on-lsp-attach".into()),
        SteelVal::BoolV(true),
    );
    assert!(
        real_editor_event.is_err(),
        "a real editor event name absent from the host's list must still be rejected"
    );
}
