// Lazy-plugin machinery tests that need no Steel plugin files on disk:
// they drive `CommandHost`/the dispatch pipeline directly, so they run on
// every platform. The plugin-loading end-to-end tests (Scheme require
// strings embed OS paths) live in `unix/plugins.rs`.

use super::*;

// ── CommandHost::register_lazy_command — collision rejection ─────────────────

/// When a lazy command's claimed name collides with an already-registered
/// name, `CommandHost::register_lazy_command` must return `Err` and NOT
/// register a `Lazy` stub for it.
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
        host.commands().register_lazy_command(
            "move-right",
            &hume_scripting::attribution::EntryId::main(plugin.clone()),
        )
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
