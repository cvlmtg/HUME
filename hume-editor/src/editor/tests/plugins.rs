// Lazy-plugin machinery tests that need no Steel plugin files on disk —
// they drive `CommandHost`/the dispatch pipeline directly, so they run on
// every platform. The plugin-loading end-to-end tests (Scheme require
// strings embed OS paths) live in `unix/plugins.rs`.

use super::*;
use crate::editor::registry::MappableCommand;
use hume_scripting::ScriptingHost;

// ── CommandHost::register_lazy_command — collision rejection ─────────────────

/// When a lazy command's claimed name collides with an already-registered
/// name, `CommandHost::register_lazy_command` must return `Err` and NOT
/// register a `Lazy` stub for it.
///
/// Flip: if collision detection were removed, the call would return `Ok` and
/// `get_mappable("move-right")` would return `Some(Lazy {..})`, shadowing the
/// built-in — both assertions would fire.
#[test]
fn lazy_stub_collision_rejected_and_stub_not_registered() {
    use hume_scripting::attribution::PluginId;
    use hume_scripting::host::EditorHost;

    let mut ed = editor_from("-[a]>b\n");
    let plugin = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    // "move-right" is a native built-in guaranteed to be in the registry.
    let result = {
        let mut host = init_host!(ed);
        host.commands().register_lazy_command("move-right", &plugin)
    };

    assert!(
        result.is_err(),
        "claiming a name already taken by a built-in must be rejected; got Ok"
    );
    assert!(
        !matches!(
            ed.state.config.registry.get_mappable("move-right"),
            Some(crate::editor::registry::MappableCommand::Lazy { .. })
        ),
        "built-in must not be shadowed by a Lazy stub after collision"
    );
}

/// Keypress dispatch of a `SteelBacked` command whose `command_table` entry
/// is missing must fail loudly, naming the desync — never silently no-op or
/// fall back to `%dispatch-command`'s own miss handling (that dispatcher is
/// reserved for `call!`/bare-name calls originating inside the VM).
///
/// This state (registry entry present, no `command_table` entry) cannot
/// arise from `define-command!` in production — it simulates a desync
/// directly to pin `call_steel_cmd`'s fail-fast guard.
///
/// Fail oracle: if `call_steel_cmd` fell back to invoking `%dispatch-command`
/// on a `command_table` miss (the pre-consolidation behavior), this would
/// silently report "unknown command" via the native/call! fallback instead
/// of naming the desync explicitly.
#[test]
fn keypress_dispatch_command_table_desync_reports_error() {
    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .config
        .registry
        .register(MappableCommand::SteelBacked {
            name: "ghost-cmd".to_owned().into(),
            doc: std::borrow::Cow::Borrowed(""),
            arity: 0,
            is_variadic: false,
            inline_output: false,
            repeatable: false,
        });
    // A fresh scripting host's command_table has no entry for "ghost-cmd" —
    // it never went through define-command!, simulating a registry/table desync.
    ed.scripting = Some(ScriptingHost::new());

    let before = state(&ed);
    ed.execute_keymap_command("ghost-cmd".into(), Some(1), false);

    assert_eq!(
        state(&ed),
        before,
        "a desync must not silently execute anything"
    );
    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == crate::editor::Severity::Error
                && e.text.contains("ghost-cmd")
                && e.text.contains("desync")
        }),
        "desync must be reported loudly; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
    );
}
