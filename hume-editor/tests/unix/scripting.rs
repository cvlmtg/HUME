use crate::mock_host::MockHost;
use hume_scripting::*;

fn host() -> ScriptingHost {
    ScriptingHost::new()
}

// ── Steel file-module isolation + prelude macro visibility ────────────────
//
// Two properties of steel-core's module system required by the plugins branch:
//  1. Private helpers are isolated across modules (foundation of plan A).
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

    let mut engine = Engine::new();
    engine
        .compile_and_run_raw_program(format!("(require \"{}\")", a_abs.display()))
        .expect("require a.scm failed");
    // Loading B last: if helpers collide, a-result would return "B".
    engine
        .compile_and_run_raw_program(format!("(require \"{}\")", b_abs.display()))
        .expect("require b.scm failed");

    let a_vals = engine
        .compile_and_run_raw_program("(a-result)".to_owned())
        .expect("a-result failed");
    let b_vals = engine
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
    // plugin.scm uses a relative require — should resolve against its own dir,
    // not the process working directory.
    std::fs::write(
        dir.path().join("plugin.scm"),
        "(require \"lib.scm\")\n(define (plugin-result) (lib-helper))\n(provide plugin-result)\n",
    )
    .unwrap();

    let plugin_abs = dir.path().join("plugin.scm").canonicalize().unwrap();

    // Process CWD is the workspace root — NOT the plugin dir.  The require
    // must still succeed because Steel resolves relative paths from the
    // requiring module's own path, not from CWD.
    let mut engine = Engine::new();
    engine
        .compile_and_run_raw_program(format!("(require \"{}\")", plugin_abs.display()))
        .expect("require plugin.scm failed");

    let vals = engine
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
/// If this test fails the prelude cannot serve plugin modules — only `init.scm`.
/// That would require documenting the limitation and NOT silently changing the
/// loader (HARD STOP per plan).
#[test]
fn global_define_syntax_is_visible_inside_required_module() {
    use steel::rvals::SteelVal;
    use steel::steel_vm::engine::Engine;

    let dir = tempfile::tempdir().unwrap();

    let mut engine = Engine::new();

    // Define a macro globally, simulating what the prelude does.
    // id-macro! is the identity macro: (id-macro! x) => x.
    engine
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

    engine
        .compile_and_run_raw_program(format!("(require \"{}\")", abs.display()))
        .expect("require failed — id-macro! not visible inside the module");

    let vals = engine
        .compile_and_run_raw_program("(get-result)".to_owned())
        .expect("get-result must be callable after require");

    assert!(
        matches!(vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "macro-expanded"),
        "id-macro! must have expanded inside the module; got {:?}",
        vals.last()
    );
}

// ── Phase 0 lazy plugin loading ───────────────────────────────────────────
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

/// `(load-plugin "user/tp")` with no keywords → plugin activates eagerly,
/// reaches `Loaded`, and its command appears in the returned defs.
#[test]
fn eager_load_no_keywords_reaches_loaded_state() {
    let (dir, init_path) = plugin_fixture(
        r#"(load-plugin "user/tp")"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("eager load must succeed");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
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

/// `(declare-plugin "user/tp" #:commands '("lazy-cmd"))` → plugin stays
/// `Declared`, body is NOT evaluated, and its commands are absent from init result.
#[test]
fn lazy_load_stays_declared_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("lazy-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("lazy load must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
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

/// `(declare-plugin "user/tp" #:commands '("my-cmd"))` → plugin stays lazy,
/// the host's `Lazy` stub for "my-cmd" maps to the plugin, body not evaluated.
#[test]
fn on_command_trigger_populates_registry_body_not_evaluated() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("#:commands declaration must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
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
        r#"(declare-plugin "user/tp" #:commands '("lazy-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

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

/// An eager plugin whose body raises an error causes `eval_init` to return
/// `Err` (fail-fast), and leaves the plugin in `Failed` state.
#[test]
fn eager_plugin_body_error_aborts_init() {
    let (dir, init_path) = plugin_fixture(
        r#"(load-plugin "user/tp")"#,
        r#"(error "intentional plugin failure")"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let result = h.eval_init(&init_path, 10_000, &mut mock, Default::default());
    assert!(
        result.is_err(),
        "init must fail when eager plugin body errors"
    );

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Failed)),
        "plugin must be Failed after body error; got {:?}",
        h.plugin_status(&id)
    );
}

// ── Phase 1 lazy plugin loading — command activations ────────────────────────
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals.

/// `#:commands '("move-right" "my-cmd")` — "move-right" clashes with a built-in →
/// colliding activation entry is dropped, a `Severity::Error` is logged, init continues with
/// the remaining valid activation entry "my-cmd".
///
/// Flip: a non-builtin name produces no Error and the activation entry is registered.
#[test]
fn manifest_collision_with_builtin_logs_error_continues() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("move-right" "my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let builtin_names: std::collections::HashSet<String> =
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
    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin must stay Declared after partial-collision #:commands list; got {:?}",
        h.plugin_status(&id)
    );

    // Flip: non-colliding entry produces no Error and is registered.
    let (dir2, init_path2) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("not-a-builtin"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h2 = host();
    h2.set_data_dir(dir2.path().to_path_buf());
    let mut mock2 = MockHost::new();

    let builtin_names2: std::collections::HashSet<String> =
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
// `lazy_stub_collision_lazy_vs_lazy_first_writer_wins` — it needs real
// `CommandRegistry` collision detection (a real `Editor` + `EditorHostImpl`),
// which `MockHost` (this file's host) deliberately does not reimplement.

/// After a lazy declare, `cmd_owners["bar"]` maps to the plugin id — not to
/// `"hume"` — even before the plugin body is evaluated.
///
/// Flip: assert it is NOT `"hume"` after the lazy declare.
#[test]
fn cmd_owners_pre_seeded_before_activation() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    // Plugin has NOT been activated yet — body was not evaluated.
    let owner = h.cmd_owners_for_test().get("bar").map(|s| s.as_str());
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
        r#"(declare-plugin "user/tp" #:commands '("my-cmd"))"#,
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

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("activate_plugin_inline must succeed");

    // Lazy stub is removed after activation.
    assert!(
        mock.lazy_command_owner("my-cmd").is_none(),
        "Lazy stub must be removed after activation"
    );
}

/// `(declare-plugin "user/tp" #:languages '("rust"))` → plugin stays lazy,
/// `activation_languages["rust"]` contains the plugin, body not evaluated.
///
/// Flip: if the `#:languages` list were not threaded through `%declare-plugin!`, the
/// plugin would stay Declared but with an empty activation_languages map.
#[test]
fn on_language_trigger_populates_registry_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("#:languages declaration must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
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
/// Flip: without the `activation_languages.retain` in the Ok branch, the activation
/// entry would survive and falsely appear pending on subsequent language sets.
#[test]
fn activate_plugin_drops_language_activation_on_loaded() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
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

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("activate_plugin_inline must succeed");

    assert!(
        h.activation_language_plugins("rust").is_empty(),
        "activation entry must be removed after activation"
    );
}

/// `(load-plugin "x")` after `(declare-plugin "x" #:commands …)` force-activates
/// the plugin: state transitions to `Loaded` and the activation command entry is cleared.
///
/// Flip: without the %activate-plugin-inline call in the load-plugin wrapper,
/// the plugin would stay `Declared` and the activation entry would remain.
#[test]
fn declare_then_load_activates_and_logs_soft_error() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        "(declare-plugin \"user/tp\" #:commands '(\"my-cmd\"))\n(load-plugin \"user/tp\")",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("declare-then-load must succeed (soft error, not hard)");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Loaded)),
        "plugin must be Loaded after explicit load-plugin; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        mock.lazy_command_owner("my-cmd").is_none(),
        "Lazy stub must be cleared after activation"
    );
    assert!(
        mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be registered after activation"
    );
    // Soft error: declare-then-load is contradictory and must be logged.
    assert!(
        h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("user/tp")
                && msg.contains("declared lazily")
        }),
        "expected a soft error about declare-then-load contradiction; got: {:?}",
        h.peek_pending_messages()
    );
}

/// `(load-plugin "foo")` then `(declare-plugin "foo" …)` — load runs first,
/// plugin is `Loaded`; the declare is ignored with a soft error.
///
/// Flip: remove the load-then-declare guard in declare_plugin and the declare
/// silently no-ops (via the existing PluginState::Declared duplicate guard) without
/// logging an error.
#[test]
fn load_then_declare_ignored_with_soft_error() {
    use hume_scripting::host::CommandHost;

    let (dir, init_path) = plugin_fixture(
        "(load-plugin \"user/tp\")\n(declare-plugin \"user/tp\" #:commands '(\"my-cmd\"))",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("load-then-declare must succeed (soft error, not hard)");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
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
                && msg.contains("already loaded")
        }),
        "expected a soft error about load-then-declare contradiction; got: {:?}",
        h.peek_pending_messages()
    );
    // The declare was ignored: no Lazy stub for "my-cmd" should be registered.
    assert!(
        mock.lazy_command_owner("my-cmd").is_none(),
        "my-cmd must not be registered as a Lazy stub — declare was ignored"
    );
}

/// `(load-plugin …)` inside an eager plugin body is rejected unconditionally —
/// even when the dep is present on disk, the gate fires before path resolution.
///
/// Flip: weaken `ensure_top_level` to also accept `EvalMode::PluginLoad` and
/// the eager in-body call succeeds instead of erroring.
#[test]
fn load_plugin_in_plugin_body_rejected() {
    // Plugin pb calls (load-plugin "user/dep") in its body; dep IS present on
    // disk so a missing-file error cannot mask the gate.
    let dir = tempfile::tempdir().unwrap();
    let pb_dir = dir.path().join("plugins").join("user").join("pb");
    let dep_dir = dir.path().join("plugins").join("user").join("dep");
    std::fs::create_dir_all(&pb_dir).unwrap();
    std::fs::create_dir_all(&dep_dir).unwrap();
    std::fs::write(pb_dir.join("plugin.scm"), r#"(load-plugin "user/dep")"#).unwrap();
    std::fs::write(dep_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin "user/pb")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let Err(msg) = h.eval_init(&init_path, 10_000, &mut mock, Default::default()) else {
        panic!("load-plugin inside a plugin body must be rejected");
    };
    assert!(
        msg.message.contains("top level") || msg.message.contains("init.scm"),
        "error must mention top-level restriction; got: {msg}"
    );
}

/// `(declare-plugin …)` inside an eager plugin body is rejected — plugins
/// cannot register other plugins; both registration verbs are top-level only.
///
/// Flip: remove the `ensure_top_level` gate from `declare_plugin` and the call
/// succeeds, silently registering a plugin from inside a plugin body.
#[test]
fn declare_plugin_in_plugin_body_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let pb_dir = dir.path().join("plugins").join("user").join("pb");
    std::fs::create_dir_all(&pb_dir).unwrap();
    std::fs::write(
        pb_dir.join("plugin.scm"),
        r#"(declare-plugin "user/other" #:commands '("other-cmd"))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin "user/pb")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let Err(msg) = h.eval_init(&init_path, 10_000, &mut mock, Default::default()) else {
        panic!("declare-plugin inside a plugin body must be rejected");
    };
    assert!(
        msg.message.contains("top level") || msg.message.contains("init.scm"),
        "error must mention top-level restriction; got: {msg}"
    );
}

// ── zero-entry / duplicate no-op regressions ─────────────────────────────────

/// `(declare-plugin "foo")` with no activation entries and no `manifest.scm`
/// on disk is a hard error even in the hume-scripting unit-test harness (no
/// editor needed) — a plugin directory without a manifest doesn't support the
/// zero-trigger form at all.
///
/// Flip: remove the manifest-presence check in `%begin-manifest-declare!` and
/// eval_source succeeds instead (silently doing nothing).
#[test]
fn declare_plugin_no_triggers_no_manifest_hard_error_scripting_level() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    // No manifest.scm written.
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(declare-plugin "user/tp")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let result = h.eval_init(&init_path, 10_000, &mut mock, Default::default());
    assert!(
        result.is_err(),
        "zero-trigger declare-plugin without a manifest.scm must hard-error"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.message.contains("manifest.scm"),
        "error must name the missing manifest.scm; got: {msg}"
    );
}

/// The Rust `%declare-plugin!` primitive's own zero-entry backstop still
/// hard-errors when called directly, bypassing the Scheme `declare-plugin`
/// wrapper's zero-trigger → manifest.scm routing.
///
/// Flip: remove the zero-entry guard in `declare_plugin` and eval_source succeeds.
#[test]
fn declare_plugin_bang_no_triggers_hard_error_scripting_level() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(%declare-plugin! "user/tp" '() '() '() (hash))"#,
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
/// Flip: check entry emptiness before collision filtering (pre-filter) and
/// this test passes with a misleading success.
#[test]
fn declare_plugin_all_commands_collide_is_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    // "move-right" is a built-in — collision filter drops it, leaving zero activation entries.
    std::fs::write(
        &init_path,
        r#"(declare-plugin "user/tp" #:commands '("move-right"))"#,
    )
    .unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let builtin_names: std::collections::HashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    let result = h.eval_init(&init_path, 10_000, &mut mock, builtin_names);
    assert!(
        result.is_err(),
        "all-collide #:commands with no other activation entry must hard-error"
    );
}

/// Duplicate `(declare-plugin …)` for the same name stays a silent no-op.
///
/// Flip: add a duplicate-declare error in LazyRegistry::declare and this errors.
#[test]
fn duplicate_declare_remains_silent_noop() {
    let (dir, init_path) = plugin_fixture(
        "(declare-plugin \"user/tp\" #:commands '(\"tp-cmd\"))\n\
         (declare-plugin \"user/tp\" #:commands '(\"tp-cmd\"))",
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

/// Duplicate `(load-plugin …)` for the same name stays a silent no-op.
///
/// Flip: add a duplicate-load error and this panics on the second load.
#[test]
fn duplicate_load_remains_silent_noop() {
    let (dir, init_path) = plugin_fixture(
        "(load-plugin \"user/tp\")\n(load-plugin \"user/tp\")",
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

