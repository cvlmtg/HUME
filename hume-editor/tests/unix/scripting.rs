use hume::testing::MockHost;
use hume_scripting::*;

fn host() -> ScriptingHost {
    ScriptingHost::new()
}

// ── Steel file-module isolation + prelude macro visibility ────────────────
//
// Two properties of steel-core's module system the plugin loading pipeline
// relies on:
//  1. Private helpers are isolated across modules.
//  2. A define-syntax macro defined globally (as the prelude does) is visible
//     inside a subsequently required module body.
//
// Not on Windows: path separators in Scheme string literals are not escaped.

#[test]
fn file_module_private_helpers_are_isolated() {
    use steel::rvals::SteelVal;
    use steel::steel_vm::engine::Engine;

    let dir = tempfile::tempdir().unwrap();

    // Two modules with the same private helper name, different return values.
    std::fs::write(
        dir.path().join("a.scm"),
        "(define (helper) \"A\")\n(define (a-result) (helper))\n(provide a-result)\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b.scm"),
        "(define (helper) \"B\")\n(define (b-result) (helper))\n(provide b-result)\n",
    )
    .unwrap();

    let a_abs = dir.path().join("a.scm").canonicalize().unwrap();
    let b_abs = dir.path().join("b.scm").canonicalize().unwrap();

    let mut steel = Engine::new();
    steel
        .compile_and_run_raw_program(format!("(require \"{}\")", a_abs.display()))
        .expect("require a.scm failed");
    // Loading B last: if helpers collide, a-result would return "B".
    steel
        .compile_and_run_raw_program(format!("(require \"{}\")", b_abs.display()))
        .expect("require b.scm failed");

    let a_vals = steel
        .compile_and_run_raw_program("(a-result)".to_owned())
        .expect("a-result failed");
    let b_vals = steel
        .compile_and_run_raw_program("(b-result)".to_owned())
        .expect("b-result failed");

    assert!(
        matches!(a_vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "A"),
        "a-result should use A's private helper (\"A\"); got {:?}",
        a_vals.last()
    );
    assert!(
        matches!(b_vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "B"),
        "b-result should use B's private helper (\"B\"); got {:?}",
        b_vals.last()
    );
}

#[test]
fn file_module_relative_require_resolves_from_module_dir() {
    use steel::rvals::SteelVal;
    use steel::steel_vm::engine::Engine;

    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("lib.scm"),
        "(define (lib-helper) \"from-lib\")\n(provide lib-helper)\n",
    )
    .unwrap();
    // plugin.scm uses a relative require, which should resolve against its own dir,
    // not the process working directory.
    std::fs::write(
        dir.path().join("plugin.scm"),
        "(require \"lib.scm\")\n(define (plugin-result) (lib-helper))\n(provide plugin-result)\n",
    )
    .unwrap();

    let plugin_abs = dir.path().join("plugin.scm").canonicalize().unwrap();

    // Process CWD is the workspace root, NOT the plugin dir.  The require
    // must still succeed because Steel resolves relative paths from the
    // requiring module's own path, not from CWD.
    let mut steel = Engine::new();
    steel
        .compile_and_run_raw_program(format!("(require \"{}\")", plugin_abs.display()))
        .expect("require plugin.scm failed");

    let vals = steel
        .compile_and_run_raw_program("(plugin-result)".to_owned())
        .expect("plugin-result failed");

    assert!(
        matches!(vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "from-lib"),
        "plugin-result should return \"from-lib\" via relative sub-require; got {:?}",
        vals.last()
    );
}

/// De-risk test for the prelude concept: a `define-syntax` macro defined in
/// a global eval (as the prelude does) must be visible inside a subsequently
/// `(require)`d module.
///
/// If this test fails the prelude cannot serve plugin modules, only `init.scm`.
/// That would require documenting the limitation and NOT silently changing the
/// loader (HARD STOP per plan).
#[test]
fn global_define_syntax_is_visible_inside_required_module() {
    use steel::rvals::SteelVal;
    use steel::steel_vm::engine::Engine;

    let dir = tempfile::tempdir().unwrap();

    let mut steel = Engine::new();

    // Define a macro globally, simulating what the prelude does.
    // id-macro! is the identity macro: (id-macro! x) => x.
    steel
        .compile_and_run_raw_program(
            "(define-syntax id-macro! (syntax-rules () ((_ x) x)))".to_owned(),
        )
        .expect("global macro definition must succeed");

    // Write a module whose top-level uses the globally-defined macro.
    // result is module-private; get-result wraps it so it can be called globally.
    std::fs::write(
        dir.path().join("mod.scm"),
        "(define result (id-macro! \"macro-expanded\"))\
         \n(define (get-result) result)\
         \n(provide get-result)\n",
    )
    .unwrap();
    let abs = dir.path().join("mod.scm").canonicalize().unwrap();

    steel
        .compile_and_run_raw_program(format!("(require \"{}\")", abs.display()))
        .expect("require failed: id-macro! not visible inside the module");

    let vals = steel
        .compile_and_run_raw_program("(get-result)".to_owned())
        .expect("get-result must be callable after require");

    assert!(
        matches!(vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "macro-expanded"),
        "id-macro! must have expanded inside the module; got {:?}",
        vals.last()
    );
}

// ── Lazy plugin loading ───────────────────────────────────────────────────
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals.

/// Helper: create a temp user plugin at `plugins/user/tp/plugin.scm` and
/// return `(TempDir, init.scm path)`.  Caller must keep TempDir alive.
fn plugin_fixture(init_body: &str, plugin_body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin_body).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, init_body).unwrap();
    (dir, init_path)
}

/// `(load-plugin! "user/tp")` with no keywords → plugin activates eagerly,
/// reaches `Loaded`, and its command appears in the returned defs.
#[test]
fn eager_load_no_keywords_reaches_loaded_state() {
    let (dir, init_path) = plugin_fixture(
        r#"(load-plugin! "user/tp")"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("eager load must succeed");

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Loaded)),
        "plugin must reach Loaded state; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be registered; got {:?}",
        mock.registered_cmds
            .iter()
            .map(|d| &d.name)
            .collect::<Vec<_>>()
    );
}

/// `(declare-plugin! "user/tp" #:commands '("lazy-cmd"))` → plugin stays
/// `Declared`, body is NOT evaluated, and its commands are absent from init result.
#[test]
fn lazy_load_stays_declared_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:commands '("lazy-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("lazy load must not error during init");

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin must stay Declared; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        !mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT be registered for a lazy plugin"
    );
}

/// `(declare-plugin! "user/tp" #:commands '("my-cmd"))` → plugin stays lazy,
/// the host's `Lazy` stub for "my-cmd" maps to the plugin, body not evaluated.
#[test]
fn on_command_trigger_populates_registry_body_not_evaluated() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:commands '("my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("#:commands declaration must not error during init");

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin declared with #:commands must stay Declared; got {:?}",
        h.plugin_status(&id)
    );
    assert_eq!(
        mock.lazy_command_owner("my-cmd"),
        Some(id.clone()),
        "the host's Lazy stub for my-cmd must map to the plugin"
    );
    assert!(
        !mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT be registered for a #:commands plugin"
    );
}

/// `activate_plugin` on a `Declared` lazy plugin → state transitions to
/// `Loaded`, returns the plugin's `SteelCmdDef`s.  Second call → idempotent
/// `Ok(vec![])`.
#[test]
fn activate_plugin_idempotent_on_declared_lazy_plugin() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:commands '("lazy-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });

    // First activation: Declared → Loaded, registers the plugin's command.
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("activate_plugin_inline must succeed");
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Loaded)),
        "plugin must be Loaded after activate_plugin_inline; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be registered after activation"
    );

    // Second activation: already Loaded → idempotent, no new registrations.
    let count_after_first = mock.registered_cmds.len();
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("second activate_plugin_inline must succeed");
    assert_eq!(
        mock.registered_cmds.len(),
        count_after_first,
        "second activation must be idempotent (no new commands registered)"
    );
}

/// An eager plugin whose body raises an error is contained: `eval_init`
/// still succeeds (so the rest of `init.scm` keeps running), the failure is
/// reported as an Error pending message, and the plugin is left in `Failed`
/// state.
#[test]
fn eager_plugin_body_error_is_contained() {
    let (dir, init_path) = plugin_fixture(
        r#"(load-plugin! "user/tp")"#,
        r#"(error "intentional plugin failure")"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("a failed plugin load must be contained, not abort eval_init");
    assert!(
        h.peek_pending_messages().iter().any(|(level, msg)| {
            matches!(level, hume_scripting::LogLevel::Error)
                && msg.contains("intentional plugin failure")
        }),
        "must log an Error carrying the plugin's own failure; got: {:?}",
        h.peek_pending_messages()
    );

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Failed)),
        "plugin must be Failed after body error; got {:?}",
        h.plugin_status(&id)
    );
}

// ── Lazy plugin loading — command activations ─────────────────────────────────
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals.

/// `#:commands '("move-right" "my-cmd")`: "move-right" clashes with a built-in →
/// colliding activation entry is dropped, a `Severity::Error` is logged, init continues with
/// the remaining valid activation entry "my-cmd".
///
/// The second half checks the control case: a non-builtin name logs no Error and its
/// activation entry is registered.
#[test]
fn manifest_collision_with_builtin_logs_error_continues() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:commands '("move-right" "my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let builtin_names: rustc_hash::FxHashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    h.eval_init(&init_path, 10_000, &mut mock, builtin_names)
        .expect("partial builtin collision must NOT abort init");

    // Error logged for the dropped activation entry.
    assert!(
        h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("move-right")
                && msg.contains("built-in")
        }),
        "expected an Error about 'move-right' conflicting with a built-in; got: {:?}",
        h.peek_pending_messages()
    );
    // Colliding activation entry not written; valid one is.
    assert!(
        mock.lazy_command_owner("move-right").is_none(),
        "colliding activation entry must not appear as a Lazy stub"
    );
    assert!(
        mock.lazy_command_owner("my-cmd").is_some(),
        "valid activation entry must appear as a Lazy stub"
    );
    // Plugin stays Declared (body not evaluated), with the remaining activation entry.
    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin must stay Declared after partial-collision #:commands list; got {:?}",
        h.plugin_status(&id)
    );

    // A non-colliding entry, by contrast, logs no Error and is registered.
    let (dir2, init_path2) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:commands '("not-a-builtin"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h2 = host();
    h2.set_data_dir(dir2.path().to_path_buf());
    let mut mock2 = MockHost::new();

    let builtin_names2: rustc_hash::FxHashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    h2.eval_init(&init_path2, 10_000, &mut mock2, builtin_names2)
        .expect("non-colliding activation entry must not error");
    assert!(
        !h2.peek_pending_messages()
            .iter()
            .any(|(sev, _)| matches!(sev, hume_scripting::LogLevel::Error)),
        "non-colliding activation entry must not log any Error"
    );
    assert!(
        mock2.lazy_command_owner("not-a-builtin").is_some(),
        "non-colliding activation entry must appear as a Lazy stub"
    );
}

// `manifest_collision_lazy_vs_lazy_logs_error_continues` moved to
// `hume-editor/src/editor/tests/plugins.rs` as
// `lazy_stub_collision_lazy_vs_lazy_first_writer_wins`; it needs real
// `CommandRegistry` collision detection (a real `Editor` + `EditorHostImpl`),
// which `MockHost` (this file's host) does not reimplement.

/// After a lazy declare, `cmd_owners["bar"]` maps to the plugin id (not to
/// `"hume"`), even before the plugin body is evaluated.
#[test]
fn cmd_owners_pre_seeded_before_activation() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    // Plugin has NOT been activated yet; body was not evaluated.
    let owners = h.cmd_owners_for_test();
    let owner = owners.get("bar").map(|s| s.as_str());
    assert!(
        owner != Some("hume"),
        "cmd_owners must be pre-seeded with the plugin id, not 'hume'; got: {:?}",
        owner
    );
    assert_eq!(
        owner,
        Some("user/tp"),
        "cmd_owners must map 'bar' to 'user/tp' before activation"
    );
}

/// `activate_plugin` drops the plugin's `Lazy` stub after the plugin body is
/// evaluated successfully.
#[test]
fn activate_plugin_drops_command_trigger_on_loaded() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:commands '("my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    // Lazy stub is present before activation.
    assert!(
        mock.lazy_command_owner("my-cmd").is_some(),
        "Lazy stub must be present before activation"
    );

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("activate_plugin_inline must succeed");

    // Lazy stub is removed after activation.
    assert!(
        mock.lazy_command_owner("my-cmd").is_none(),
        "Lazy stub must be removed after activation"
    );
}

/// `(declare-plugin! "user/tp" #:languages '("rust"))` → plugin stays lazy,
/// `activation_languages["rust"]` contains the plugin, body not evaluated.
///
/// If `%declare-plugin!` dropped the `#:languages` list, the plugin would still be
/// Declared but its activation_languages map would be empty.
#[test]
fn on_language_trigger_populates_registry_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:languages '("rust"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("#:languages declaration must not error during init");

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin declared with #:languages must stay Declared; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        h.activation_language_plugins("rust").contains(&id),
        "activation_languages must map \"rust\" to the plugin"
    );
    assert!(
        !mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT be registered for a #:languages plugin"
    );
}

/// `activate_plugin` on a language-matched plugin drops the activation entry on success.
///
/// The `activation_languages.retain` in the Ok branch does the removal; if it were
/// skipped, the entry would look pending on every later language set.
#[test]
fn activate_plugin_drops_language_activation_on_loaded() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin! "user/tp" #:languages '("rust"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    assert!(
        !h.activation_language_plugins("rust").is_empty(),
        "activation entry must be present before activation"
    );

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("activate_plugin_inline must succeed");

    assert!(
        h.activation_language_plugins("rust").is_empty(),
        "activation entry must be removed after activation"
    );
}

/// `(load-plugin! "x")` after `(declare-plugin! "x" #:commands …)` leaves the
/// plugin lazy: the declared triggers stay and the body waits for one of them.
///
/// The user's declaration comes first and `load-plugin!` only stores the
/// config, so no activation runs and nothing is logged.
#[test]
fn declare_then_load_keeps_the_plugin_lazy() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        "(declare-plugin! \"user/tp\" #:commands '(\"my-cmd\"))\n(load-plugin! \"user/tp\")",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("declare-then-load must succeed");

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin must stay Declared after load-plugin!; got {:?}",
        h.plugin_status(&id)
    );
    assert_eq!(
        mock.lazy_command_owner("my-cmd"),
        Some(id),
        "the declared Lazy stub must stay in place"
    );
    assert!(
        !mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "the body must not have run"
    );
    assert!(
        h.peek_pending_messages().is_empty(),
        "declare-then-load is not a contradiction; got: {:?}",
        h.peek_pending_messages()
    );
}

/// `(load-plugin! "foo")` then `(declare-plugin! "foo" …)`: load runs first,
/// plugin is `Loaded`; the declare is ignored with a soft error.
///
/// That error comes from the load-then-declare guard in `declare_plugin`. Without
/// it the declare would fall through to the first-wins guard and no-op with
/// nothing logged.
#[test]
fn load_then_declare_ignored_with_soft_error() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        "(load-plugin! \"user/tp\")\n(declare-plugin! \"user/tp\" #:commands '(\"my-cmd\"))",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("load-then-declare must succeed (soft error, not hard)");

    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Loaded)),
        "plugin must remain Loaded; got {:?}",
        h.plugin_status(&id)
    );
    // Soft error: the declare after load is contradictory and must be logged.
    assert!(
        h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("user/tp")
                && msg.contains("comes after its load-plugin!")
        }),
        "expected a soft error about load-then-declare contradiction; got: {:?}",
        h.peek_pending_messages()
    );
    // The declare was ignored: no Lazy stub for "my-cmd" should be registered.
    assert!(
        mock.lazy_command_owner("my-cmd").is_none(),
        "my-cmd must not be registered as a Lazy stub, declare was ignored"
    );
}

/// `(load-plugin! "x")` then `(declare-plugin! "x" …)` for a plugin that ships a
/// manifest: the manifest's entries are already declared, the user's declare is
/// ignored, and the ignored declare is reported.
#[test]
fn load_then_declare_on_lazy_plugin_is_reported() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        "(load-plugin! \"user/tp\")\n(declare-plugin! \"user/tp\" #:commands '(\"my-cmd\"))",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    std::fs::write(
        dir.path()
            .join("plugins")
            .join("user")
            .join("tp")
            .join("manifest.scm"),
        "(declare-plugin! \"user/tp\" #:commands '(\"tp-cmd\"))",
    )
    .unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("load-then-declare must succeed (soft error, not hard)");

    assert!(
        h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("user/tp")
                && msg.contains("comes after its load-plugin!")
        }),
        "expected an error about the declare after load-plugin!; got: {:?}",
        h.peek_pending_messages()
    );
    assert!(
        mock.lazy_command_owner("my-cmd").is_none(),
        "my-cmd must not be registered as a Lazy stub, declare was ignored"
    );
}

/// `(load-plugin! …)` inside an eager plugin body is rejected unconditionally,
/// even when the dep is present on disk, the gate fires before path
/// resolution. `pb`'s own activation is contained by the rejection (a body
/// error like any other), so `eval_init` still succeeds; `pb` itself ends up
/// `Failed`.
///
/// Were `ensure_top_level` to accept `EvalMode::PluginLoad` too, the eager
/// in-body call would succeed.
#[test]
fn load_plugin_in_plugin_body_rejected() {
    // Plugin pb calls (load-plugin! "user/dep") in its body; dep IS present on
    // disk so a missing-file error cannot mask the gate.
    let dir = tempfile::tempdir().unwrap();
    let pb_dir = dir.path().join("plugins").join("user").join("pb");
    let dep_dir = dir.path().join("plugins").join("user").join("dep");
    std::fs::create_dir_all(&pb_dir).unwrap();
    std::fs::create_dir_all(&dep_dir).unwrap();
    std::fs::write(pb_dir.join("plugin.scm"), r#"(load-plugin! "user/dep")"#).unwrap();
    std::fs::write(dep_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin! "user/pb")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("a failed plugin load must be contained, not abort eval_init");
    assert!(
        h.peek_pending_messages().iter().any(|(level, msg)| {
            matches!(level, hume_scripting::LogLevel::Error)
                && (msg.contains("top level") || msg.contains("init.scm"))
        }),
        "error must mention top-level restriction; got: {:?}",
        h.peek_pending_messages()
    );
    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "pb".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Failed)),
        "pb must be Failed after its body's top-level violation; got {:?}",
        h.plugin_status(&id)
    );
}

/// `(declare-plugin! …)` inside an eager plugin body is rejected: plugins
/// cannot register other plugins; both registration verbs are top-level
/// only. `pb`'s own activation is contained by the rejection (a body error
/// like any other), so `eval_init` still succeeds; `pb` itself ends up
/// `Failed`.
///
/// The `ensure_top_level` gate in `declare_plugin` is what stops a plugin body
/// from quietly registering another plugin.
#[test]
fn declare_plugin_in_plugin_body_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let pb_dir = dir.path().join("plugins").join("user").join("pb");
    std::fs::create_dir_all(&pb_dir).unwrap();
    std::fs::write(
        pb_dir.join("plugin.scm"),
        r#"(declare-plugin! "user/other" #:commands '("other-cmd"))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin! "user/pb")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("a failed plugin load must be contained, not abort eval_init");
    assert!(
        h.peek_pending_messages().iter().any(|(level, msg)| {
            matches!(level, hume_scripting::LogLevel::Error)
                && (msg.contains("top level") || msg.contains("init.scm"))
        }),
        "error must mention top-level restriction; got: {:?}",
        h.peek_pending_messages()
    );
    let id = attribution::EntryId::main(attribution::PluginId::User {
        user: "user".to_string(),
        repo: "pb".to_string(),
    });
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Failed)),
        "pb must be Failed after its body's top-level violation; got {:?}",
        h.plugin_status(&id)
    );
}

// ── zero-entry / duplicate no-op regressions ─────────────────────────────────

/// `(declare-plugin! "foo")` with no activation entries is a hard error even in
/// the hume-scripting unit-test harness (no editor needed): an entry with no
/// trigger could never activate.
#[test]
fn declare_plugin_without_triggers_hard_error_scripting_level() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    // No manifest.scm written.
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(declare-plugin! "user/tp")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let result = h.eval_init(&init_path, 10_000, &mut mock, Default::default());
    assert!(
        result.is_err(),
        "declare-plugin! without triggers must hard-error"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.message.contains("no activation entries"),
        "error must name the missing triggers; got: {msg}"
    );
}

/// The Rust `%declare-plugin!` primitive hard-errors on its own when called
/// directly with no activation entries, bypassing the Scheme wrapper.
///
/// Without the zero-entry guard in `declare_plugin`, eval_source would succeed.
#[test]
fn declare_plugin_bang_no_triggers_hard_error_scripting_level() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(%declare-plugin! "user/tp" "plugin.scm" '() '() '() '())"#,
    )
    .unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let result = h.eval_init(&init_path, 10_000, &mut mock, Default::default());
    assert!(
        result.is_err(),
        "%declare-plugin! with no activation entries must hard-error"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.message.contains("no activation entries") || msg.message.contains("never be activated"),
        "error must describe the zero-entry problem; got: {msg}"
    );
}

/// `#:commands` names that ALL collide with builtins leave zero effective
/// activation entries → hard error (the post-filter zero-entry check fires).
///
/// The emptiness check has to run after collision filtering. Checked before it,
/// the list still holds one name and the declare would wrongly succeed.
#[test]
fn declare_plugin_all_commands_collide_is_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    // "move-right" is a built-in, so the collision filter drops it, leaving no activation entries.
    std::fs::write(
        &init_path,
        r#"(declare-plugin! "user/tp" #:commands '("move-right"))"#,
    )
    .unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let builtin_names: rustc_hash::FxHashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    let result = h.eval_init(&init_path, 10_000, &mut mock, builtin_names);
    assert!(
        result.is_err(),
        "all-collide #:commands with no other activation entry must hard-error"
    );
}

/// Duplicate `(declare-plugin! …)` for the same name stays a silent no-op.
///
/// `LazyRegistry::declare` must not treat the repeat as an error.
#[test]
fn duplicate_declare_remains_silent_noop() {
    let (dir, init_path) = plugin_fixture(
        "(declare-plugin! \"user/tp\" #:commands '(\"tp-cmd\"))\n\
         (declare-plugin! \"user/tp\" #:commands '(\"tp-cmd\"))",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("duplicate declare must be a silent no-op, not an error");

    // No error-level message about the duplicate declare.
    assert!(
        !h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error) && msg.contains("user/tp")
        }),
        "duplicate declare must not log an error; got: {:?}",
        h.peek_pending_messages()
    );
}

/// Duplicate `(load-plugin! …)` for the same name stays a silent no-op.
///
/// The second load must neither error nor panic.
#[test]
fn duplicate_load_remains_silent_noop() {
    let (dir, init_path) = plugin_fixture(
        "(load-plugin! \"user/tp\")\n(load-plugin! \"user/tp\")",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("duplicate load must be a silent no-op, not an error");

    // No error-level message about the duplicate load.
    assert!(
        !h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error) && msg.contains("user/tp")
        }),
        "duplicate load must not log an error; got: {:?}",
        h.peek_pending_messages()
    );
}
