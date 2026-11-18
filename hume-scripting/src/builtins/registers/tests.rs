use super::*;
use crate::testing::test_support::SteelCtxTestHarness;
use hume_platform::dirs::Dirs;

#[test]
fn register_arg_rejects_multi_char_name() {
    let err = register_arg("12").unwrap_err();
    assert!(
        err.to_string()
            .contains("expected a single-character register name"),
        "got: {err}"
    );
}

#[test]
fn register_arg_rejects_empty_name() {
    let err = register_arg("").unwrap_err();
    assert!(
        err.to_string()
            .contains("expected a single-character register name"),
        "got: {err}"
    );
}

#[test]
fn register_arg_accepts_digit_and_special_names() {
    for name in ["0", "9", "k", "c", "b"] {
        assert_eq!(register_arg(name).unwrap(), name.chars().next().unwrap());
    }
}

#[test]
fn register_arg_rejects_macro_and_search() {
    for name in ["q", "s"] {
        let err = register_arg(name).unwrap_err().to_string();
        assert!(
            err.contains(&format!("invalid register '{name}'")),
            "got: {err}"
        );
    }
}

#[test]
fn register_arg_message_lists_the_valid_registers() {
    let err = register_arg("x").unwrap_err().to_string();
    assert!(
        err.contains("invalid register 'x'; valid: 0-9, k, c, b"),
        "got: {err}"
    );
}

#[test]
fn register_name_answers_true_only_for_names_read_register_takes() {
    for name in ["0", "9", "k", "c", "b"] {
        let answer = register_name(name.to_string()).unwrap();
        assert!(matches!(answer, SteelVal::BoolV(true)), "{name:?}");
        assert!(register_arg(name).is_ok());
    }
    for name in ["q", "s", "f", "p", "ab", ""] {
        let answer = register_name(name.to_string()).unwrap();
        assert!(matches!(answer, SteelVal::BoolV(false)), "{name:?}");
        assert!(register_arg(name).is_err());
    }
}

/// `NullHost` has no `RegisterHost` capability: both builtins must surface
/// the standard "not supported by this host" error, same as any other
/// optional-capability builtin (`goto-location!`, `show-popup!`, …). `"3"` is
/// a valid name, so the failure is the capability check.
#[test]
fn read_register_without_capability_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = read_register(&mut ctx, "3".to_string()).unwrap_err();
    assert!(
        err.to_string().contains("not supported by this host"),
        "got: {err}"
    );
}

#[test]
fn write_register_without_capability_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = write_register(
        &mut ctx,
        "3".to_string(),
        SteelVal::ListV(vec![SteelVal::StringV("hi".into())].into()),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("not supported by this host"),
        "got: {err}"
    );
}

/// A bare string is rejected before the host capability is even consulted.
/// Register-name validation and value-shape validation are independent
/// checks, but this proves the value check fires against a capable host too,
/// not just as a side effect of a missing capability.
#[test]
fn write_register_rejects_bare_string_value() {
    let mut host = RegisterCapableHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    let err =
        write_register(&mut ctx, "3".to_string(), SteelVal::StringV("hi".into())).unwrap_err();
    assert!(err.to_string().contains("expected a list"), "got: {err}");
}

/// End-to-end through the real registration table: `write-register!` then
/// `read-register` on a host that actually stores values, proving the two
/// builtins agree on the wire shape (a list of strings both ways). Ends in
/// `log!` because `eval_source` reports only success/failure, the same
/// round-trip-via-log idiom as `fs::tests::data_dir_resolves_through_real_registration`.
#[test]
fn write_then_read_round_trips_through_real_registration() {
    let mut host = crate::ScriptingHost::for_testing(&Dirs::none());
    let mut backing = RegisterCapableHost::default();
    host.eval_source(
        r#"(write-register! "3" (list "alpha" "beta"))
           (log! 'info (car (read-register "3")))
           (log! 'info (cadr (read-register "3")))"#,
        &mut backing,
    )
    .expect("write-register!/read-register must round-trip through the real registration table");
    let msgs = host.take_pending_messages();
    assert!(msgs.iter().any(|(_, m)| m == "alpha"), "got: {msgs:?}");
    assert!(msgs.iter().any(|(_, m)| m == "beta"), "got: {msgs:?}");
}

// ── Test hosts ────────────────────────────────────────────────────────────────

/// [`crate::testing::null_host::NullHost`] backed by a real in-memory
/// register store, enough to prove `write-register!`/`read-register`
/// round-trip correctly without pulling in a real `Editor`.
#[derive(Default)]
struct RegisterCapableHost {
    inner: crate::testing::null_host::NullHost,
    store: std::collections::HashMap<char, Vec<String>>,
}

impl crate::host::EditorHost for RegisterCapableHost {
    fn cursor(&mut self) -> &mut dyn crate::host::CursorHost {
        self.inner.cursor()
    }
    fn commands(&mut self) -> &mut dyn crate::host::CommandHost {
        self.inner.commands()
    }
    fn language(&mut self) -> &mut dyn crate::host::LanguageHost {
        self.inner.language()
    }
    fn settings(&mut self) -> &mut dyn crate::host::SettingsHost {
        self.inner.settings()
    }
    fn buffers(&mut self) -> &mut dyn crate::host::BufferHost {
        self.inner.buffers()
    }
    fn events(&mut self) -> &mut dyn crate::host::EventHost {
        self.inner.events()
    }
    fn permit_plugin_activation(
        &mut self,
        plugin: &crate::attribution::EntryId,
    ) -> Result<(), String> {
        self.inner.permit_plugin_activation(plugin)
    }
    fn registers(&mut self) -> Option<&mut dyn crate::host::RegisterHost> {
        Some(self)
    }
}

impl crate::host::RegisterHost for RegisterCapableHost {
    fn read_register(&mut self, name: char) -> Option<Vec<String>> {
        self.store.get(&name).cloned()
    }
    fn write_register(&mut self, name: char, values: Vec<String>) {
        self.store.insert(name, values);
    }
}
