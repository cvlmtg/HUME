use super::*;
use crate::test_support::SteelCtxTestHarness;

fn noop_proc(_args: &[SteelVal]) -> SteelResult {
    Ok(SteelVal::Void)
}

/// `register-hook!` errors when the first argument is not a symbol.
#[test]
fn register_hook_non_symbol_arg_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = register_hook(
        &mut ctx,
        SteelVal::StringV("on-buffer-open".into()), // should be a symbol
        SteelVal::FuncV(noop_proc),
    );
    let msg = result
        .expect_err("register-hook! must reject a string name")
        .to_string();
    assert!(msg.contains("expected an event-name symbol"), "got: {msg}");
}

/// `register-hook!` rejects a handler that is not callable at registration,
/// not later when the event fires.
#[test]
fn register_hook_non_callable_proc_errors() {
    let mut h = SteelCtxTestHarness::new();
    let msg = register_hook(
        &mut h.ctx_init(),
        SteelVal::SymbolV("on-buffer-open".into()),
        SteelVal::IntV(1),
    )
    .expect_err("a non-callable proc must be rejected")
    .to_string();
    assert!(msg.contains("expected a callable"), "got: {msg}");
}

/// `register-hook!` errors for an unknown hook name.
///
/// The `known_event_names()` guard keeps a typo from registering a hook that
/// never fires.
#[test]
fn register_hook_unknown_hook_name_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = register_hook(
        &mut ctx,
        SteelVal::SymbolV("on-nonexistent-event".into()),
        SteelVal::FuncV(noop_proc),
    );
    assert!(
        result.is_err(),
        "register-hook! must reject unknown hook names"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("unknown event"),
        "error must mention 'unknown event'; got: {msg}"
    );
    assert!(
        msg.contains("on-buffer-open"),
        "error must list the valid names (from NULL_HOST_EVENT_NAMES); got: {msg}"
    );
}

/// `register-hook!` in init mode with a valid name registers the handler.
#[test]
fn register_hook_valid_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = register_hook(
            &mut ctx,
            SteelVal::SymbolV("on-buffer-save".into()),
            SteelVal::FuncV(noop_proc),
        );
        assert!(result.is_ok(), "register-hook! must succeed in init mode");
    }
    assert_eq!(
        h.registries.hooks.handlers_for("on-buffer-save").len(),
        1,
        "one handler must be registered for on-buffer-save"
    );
    assert_eq!(
        *h.registries.hooks.handlers_for("on-buffer-save")[0]
            .handler
            .provenance()
            .owner,
        crate::attribution::Owner::User,
        "a top-level (non-plugin) registration is the user's"
    );
}

/// `register-hook!` is also valid during plugin activation (a plugin body running).
#[test]
fn register_hook_valid_during_plugin_load() {
    use crate::attribution::{EntryId, PluginId};
    let mut h = SteelCtxTestHarness::new();
    // Simulate being inside a plugin body.
    h.attribution
        .push_body(EntryId::main(PluginId::parse("core:myplugin").unwrap()));
    {
        let mut ctx = h.ctx(); // EvalMode::PluginActivation → allowed
        let result = register_hook(
            &mut ctx,
            SteelVal::SymbolV("on-buffer-open".into()),
            SteelVal::FuncV(noop_proc),
        );
        assert!(
            result.is_ok(),
            "register-hook! must succeed during plugin load"
        );
    }
    assert!(h.registries.hooks.has_match("on-buffer-open", None));
    assert_eq!(
        *h.registries.hooks.handlers_for("on-buffer-open")[0]
            .handler
            .provenance()
            .owner,
        crate::attribution::Owner::Plugin(EntryId::main(PluginId::parse("core:myplugin").unwrap())),
        "a plugin-body registration must be attributed to the currently-executing plugin"
    );
    assert_eq!(
        h.registries.hooks.handlers_for("on-buffer-open")[0]
            .handler
            .provenance()
            .behalf,
        hume_core::ConfigOrigin::Plugin,
        "a plugin's hook runs on the plugin's behalf, as nobody asked for it"
    );
}

/// Validation is host-driven, not a compiled-in table:
/// `NullHost`'s known-names fixture diverges from the editor's
/// real event set (see `NULL_HOST_EVENT_NAMES`'s doc comment): it includes
/// a synthetic `on-stub-only` name the editor never defines, and omits real
/// editor events like `on-lsp-attach`. `register-hook!` must follow the
/// host's list exactly in both directions.
#[test]
fn register_hook_validates_against_the_host_not_a_compiled_in_table() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();

    let stub_only = register_hook(
        &mut ctx,
        SteelVal::SymbolV("on-stub-only".into()),
        SteelVal::FuncV(noop_proc),
    );
    assert!(
        stub_only.is_ok(),
        "a name absent from the editor but present on the host must be accepted"
    );

    let real_editor_event = register_hook(
        &mut ctx,
        SteelVal::SymbolV("on-lsp-attach".into()),
        SteelVal::FuncV(noop_proc),
    );
    assert!(
        real_editor_event.is_err(),
        "a real editor event name absent from the host's list must still be rejected"
    );
}

mod held {
    use hume_platform::dirs::Dirs;
    use steel::rvals::SteelVal;

    use crate::host::{
        BufferHost, CommandHost, CursorHost, EditorHost, EventHost, LanguageHost, SettingsHost,
    };
    use crate::null_host::{HELD_EVENT, NullHost};
    use crate::{EvalError, ScriptingHost};
    use hume_core::HostToken;

    /// A [`NullHost`] whose event capability keeps the holds open and the
    /// ones finished, as the editor's does for a parked write.
    #[derive(Default)]
    struct HoldHost {
        inner: NullHost,
        finished: Vec<HostToken>,
    }

    impl EditorHost for HoldHost {
        fn cursor(&mut self) -> &mut dyn CursorHost {
            &mut self.inner
        }
        fn commands(&mut self) -> &mut dyn CommandHost {
            &mut self.inner
        }
        fn language(&mut self) -> &mut dyn LanguageHost {
            &mut self.inner
        }
        fn settings(&mut self) -> &mut dyn SettingsHost {
            &mut self.inner
        }
        fn buffers(&mut self) -> &mut dyn BufferHost {
            &mut self.inner
        }
        fn events(&mut self) -> &mut dyn EventHost {
            self
        }
        fn permit_plugin_activation(
            &mut self,
            plugin: &crate::attribution::EntryId,
        ) -> Result<(), String> {
            self.inner.permit_plugin_activation(plugin)
        }
    }

    impl EventHost for HoldHost {
        fn known_event_names(&self) -> &'static [&'static str] {
            self.inner.known_event_names()
        }
        fn held_event_arg_count(&self, name: &str) -> Option<usize> {
            self.inner.held_event_arg_count(name)
        }
        fn finish_hold(&mut self, hold: HostToken) {
            self.finished.push(hold);
        }
    }

    fn register_for(event: &str, handler: &str) -> Result<ScriptingHost, String> {
        let mut host = ScriptingHost::new(&Dirs::none());
        host.eval_source(
            &format!("(register-hook! '{event} {handler})"),
            &mut NullHost,
        )?;
        Ok(host)
    }

    fn register(handler: &str) -> Result<ScriptingHost, String> {
        register_for(HELD_EVENT, handler)
    }

    /// Fires the held event the way the editor does: one hold per handler
    /// that takes `done`, minted before any handler runs.
    fn fire(
        host: &mut ScriptingHost,
        hosts: &mut HoldHost,
    ) -> (Vec<HostToken>, Result<(), EvalError>) {
        let handlers = host.held_handlers(HELD_EVENT);
        let holds: Vec<HostToken> = (0..handlers.holds_needed())
            .map(|_| HostToken::mint())
            .collect();
        let result = host
            .fire_held_hook(handlers, &[SteelVal::IntV(0)], holds.clone(), hosts)
            .map(|_| ());
        (holds, result)
    }

    /// The event has one arg, so a handler takes one parameter, or two to
    /// take `done`. Nothing else can be told apart at registration.
    #[test]
    fn held_event_handler_takes_the_event_args_with_or_without_done() {
        assert!(register("(lambda (pane) 0)").is_ok());
        assert!(register("(lambda (pane done) 0)").is_ok());
        for refused in [
            "(lambda () 0)",
            "(lambda (pane done extra) 0)",
            "(lambda args 0)",
            "(lambda (pane . rest) 0)",
            "+",
        ] {
            let Err(msg) = register(refused) else {
                panic!("{refused} must be refused");
            };
            assert!(msg.contains("handler is a lambda of 1 parameters"), "{msg}");
        }
    }

    #[test]
    fn only_a_held_event_handler_taking_done_needs_a_hold() {
        let host = register("(lambda (pane) 0)").unwrap();
        assert_eq!(host.held_handlers(HELD_EVENT).holds_needed(), 0);
        let host = register("(lambda (pane done) 0)").unwrap();
        assert_eq!(host.held_handlers(HELD_EVENT).holds_needed(), 1);
        let host = register_for("on-buffer-save", "(lambda (pane extra) 0)").unwrap();
        assert_eq!(host.held_handlers("on-buffer-save").holds_needed(), 0);
    }

    #[test]
    fn handler_without_done_takes_no_hold() {
        let mut host = register("(lambda (pane) 0)").unwrap();
        let mut hosts = HoldHost::default();
        let (holds, result) = fire(&mut host, &mut hosts);
        assert!(result.is_ok());
        assert!(holds.is_empty());
        assert!(hosts.finished.is_empty());
    }

    #[test]
    fn handler_calling_done_finishes_its_hold_through_the_host() {
        let mut host = register("(lambda (pane done) (done))").unwrap();
        let mut hosts = HoldHost::default();
        let (holds, result) = fire(&mut host, &mut hosts);
        assert!(result.is_ok());
        assert_eq!(holds.len(), 1);
        assert_eq!(hosts.finished, holds);
    }

    #[test]
    fn handler_keeping_done_leaves_its_hold_open() {
        let mut host = register("(lambda (pane done) 0)").unwrap();
        let mut hosts = HoldHost::default();
        let (holds, result) = fire(&mut host, &mut hosts);
        assert!(result.is_ok());
        assert_eq!(holds.len(), 1);
        assert!(hosts.finished.is_empty());
    }

    #[test]
    fn done_with_an_argument_raises() {
        let mut host = register("(lambda (pane done) (done 'bogus))").unwrap();
        let mut hosts = HoldHost::default();
        let (holds, result) = fire(&mut host, &mut hosts);
        assert!(result.is_err(), "done takes no argument");
        assert_eq!(hosts.finished, holds, "a raising handler releases its hold");
    }

    #[test]
    fn raising_handler_finishes_its_own_hold_and_reports() {
        let mut host = register(r#"(lambda (pane done) (error "boom"))"#).unwrap();
        let mut hosts = HoldHost::default();
        let (holds, result) = fire(&mut host, &mut hosts);
        assert!(result.is_err());
        assert_eq!(holds.len(), 1);
        assert_eq!(hosts.finished, holds);
    }

    #[test]
    #[should_panic(expected = "fire_held_hook")]
    fn a_handler_taking_done_is_never_fired_as_a_plain_hook() {
        let mut host = register("(lambda (pane done) 0)").unwrap();
        let result = host.fire_hook(HELD_EVENT, None, &[SteelVal::IntV(0)], &mut NullHost);
        assert!(result.is_ok(), "{:?}", result.err().map(|e| e.message));
    }
}
