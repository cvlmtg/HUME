use super::*;
use crate::test_support::SteelCtxTestHarness;

#[test]
fn register_arg_rejects_multi_char_name() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = register_arg(&mut ctx, "12", "read-register").unwrap_err();
    assert!(
        err.to_string()
            .contains("expected a single-character register name"),
        "got: {err}"
    );
}

#[test]
fn register_arg_rejects_empty_name() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = register_arg(&mut ctx, "", "read-register").unwrap_err();
    assert!(
        err.to_string()
            .contains("expected a single-character register name"),
        "got: {err}"
    );
}

/// `NullHost::is_valid_register_name` always answers `false` (no registry to
/// validate against), so every single-char name — including a real one like
/// `'3'` — surfaces as "invalid" through it. That's sufficient to prove the
/// rejection path fires and is worded correctly; the accept path needs a host
/// with real validation logic, below.
#[test]
fn register_arg_rejects_whatever_the_host_calls_invalid() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    for name in ["3", "k", "c", "b", "q", "s", "a"] {
        let err = register_arg(&mut ctx, name, "read-register").unwrap_err();
        assert!(
            err.to_string().contains("invalid register"),
            "name {name:?} got: {err}"
        );
    }
}

#[test]
fn register_arg_accepts_digit_and_special_names() {
    let mut host = RegisterCapableHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    for name in ["0", "9", "k", "c", "b"] {
        assert_eq!(
            register_arg(&mut ctx, name, "read-register").unwrap(),
            name.chars().next().unwrap()
        );
    }
}

#[test]
fn register_arg_rejects_macro_and_search_even_with_real_validation() {
    let mut host = RegisterCapableHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    for name in ["q", "s"] {
        let err = register_arg(&mut ctx, name, "read-register").unwrap_err();
        assert!(
            err.to_string().contains("invalid register"),
            "name {name:?} got: {err}"
        );
    }
}

/// `NullHost` has no `RegisterHost` capability — both builtins must surface
/// the standard "not supported by this host" error, same as any other
/// optional-capability builtin (`goto-location!`, `show-popup!`, …). Uses
/// `ValidNameHost` (real name validation, no registers) rather than
/// `NullHost` directly, so the failure is provably the capability check and
/// not `register_arg` rejecting the name first.
#[test]
fn read_register_without_capability_errors() {
    let mut host = ValidNameHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    let err = read_register(&mut ctx, "3".to_string()).unwrap_err();
    assert!(
        err.to_string().contains("not supported by this host"),
        "got: {err}"
    );
}

#[test]
fn write_register_without_capability_errors() {
    let mut host = ValidNameHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
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

/// A bare string is rejected before the host capability is even consulted —
/// register-name validation and value-shape validation are independent
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
/// `log!` because `eval_source` reports only success/failure — same
/// round-trip-via-log idiom as `fs::tests::data_dir_resolves_through_real_registration`.
#[test]
fn write_then_read_round_trips_through_real_registration() {
    let mut host = crate::ScriptingHost::new();
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

/// Register names this crate's builtins must accept — mirrors
/// `hume_ops::register::is_valid_register_name` without depending on
/// `hume-ops` (a crate this layer intentionally doesn't pull in; see
/// `crate::host::EditorHost`'s module doc on the dependency wall). The real
/// constant is exercised end-to-end by `hume-editor`'s integration test.
fn valid_test_register_name(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, 'k' | 'c' | 'b')
}

/// [`crate::null_host::NullHost`] wrapper with real register-name validation
/// but no [`crate::host::RegisterHost`] capability — isolates "capability
/// missing" from "name rejected" in the tests above.
#[derive(Default)]
struct ValidNameHost {
    inner: crate::null_host::NullHost,
}

impl crate::host::EditorHost for ValidNameHost {
    fn cursor(&mut self) -> &mut dyn crate::host::CursorHost {
        &mut self.inner
    }
    fn commands(&mut self) -> &mut dyn crate::host::CommandHost {
        self
    }
    fn language(&mut self) -> &mut dyn crate::host::LanguageHost {
        &mut self.inner
    }
    fn settings(&mut self) -> &mut dyn crate::host::SettingsHost {
        &mut self.inner
    }
    fn buffers(&mut self) -> &mut dyn crate::host::BufferHost {
        &mut self.inner
    }
    fn events(&mut self) -> &mut dyn crate::host::EventHost {
        &mut self.inner
    }
}

impl crate::host::CommandHost for ValidNameHost {
    fn is_valid_register_name(&self, ch: char) -> bool {
        valid_test_register_name(ch)
    }
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.inner.command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        self.inner.run_command_sync(name, count, extend, register)
    }
    fn register_command(&mut self, def: crate::types::SteelCmdDef) -> Result<(), String> {
        self.inner.register_command(def)
    }
    fn register_typed_command(
        &mut self,
        def: crate::types::SteelTypedCmdDef,
    ) -> Result<(), String> {
        self.inner.register_typed_command(def)
    }
    fn unregister_command(&mut self, name: &str) {
        self.inner.unregister_command(name)
    }
    fn register_lazy_command(
        &mut self,
        name: &str,
        plugin: &crate::attribution::PluginId,
    ) -> Result<(), String> {
        self.inner.register_lazy_command(name, plugin)
    }
    fn register_lazy_typed_command(
        &mut self,
        name: &str,
        plugin: &crate::attribution::PluginId,
    ) -> Result<(), String> {
        self.inner.register_lazy_typed_command(name, plugin)
    }
    fn lazy_command_owner(&self, name: &str) -> Option<crate::attribution::PluginId> {
        self.inner.lazy_command_owner(name)
    }
    fn unregister_lazy_stubs_of(&mut self, plugin: &crate::attribution::PluginId) {
        self.inner.unregister_lazy_stubs_of(plugin)
    }
}

/// Like [`ValidNameHost`] but backed by a real in-memory register store —
/// enough to prove `write-register!`/`read-register` round-trip correctly
/// without pulling in a real `Editor`. Wraps `ValidNameHost` rather than
/// `NullHost` directly so the `CommandHost` delegation (identical to
/// `ValidNameHost`'s own) isn't duplicated a second time.
#[derive(Default)]
struct RegisterCapableHost {
    inner: ValidNameHost,
    store: std::collections::HashMap<char, Vec<String>>,
}

impl crate::host::EditorHost for RegisterCapableHost {
    fn cursor(&mut self) -> &mut dyn crate::host::CursorHost {
        self.inner.cursor()
    }
    fn commands(&mut self) -> &mut dyn crate::host::CommandHost {
        self
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
    fn registers(&mut self) -> Option<&mut dyn crate::host::RegisterHost> {
        Some(self)
    }
}

impl crate::host::CommandHost for RegisterCapableHost {
    fn is_valid_register_name(&self, ch: char) -> bool {
        self.inner.is_valid_register_name(ch)
    }
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.inner.command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        self.inner.run_command_sync(name, count, extend, register)
    }
    fn register_command(&mut self, def: crate::types::SteelCmdDef) -> Result<(), String> {
        self.inner.register_command(def)
    }
    fn register_typed_command(
        &mut self,
        def: crate::types::SteelTypedCmdDef,
    ) -> Result<(), String> {
        self.inner.register_typed_command(def)
    }
    fn unregister_command(&mut self, name: &str) {
        self.inner.unregister_command(name)
    }
    fn register_lazy_command(
        &mut self,
        name: &str,
        plugin: &crate::attribution::PluginId,
    ) -> Result<(), String> {
        self.inner.register_lazy_command(name, plugin)
    }
    fn register_lazy_typed_command(
        &mut self,
        name: &str,
        plugin: &crate::attribution::PluginId,
    ) -> Result<(), String> {
        self.inner.register_lazy_typed_command(name, plugin)
    }
    fn lazy_command_owner(&self, name: &str) -> Option<crate::attribution::PluginId> {
        self.inner.lazy_command_owner(name)
    }
    fn unregister_lazy_stubs_of(&mut self, plugin: &crate::attribution::PluginId) {
        self.inner.unregister_lazy_stubs_of(plugin)
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
