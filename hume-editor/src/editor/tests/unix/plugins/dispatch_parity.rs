//! One registry, one dispatcher: lazy-activation dispatch parity, and
//! runtime/init.scm.example coverage.

use super::*;
use crate::editor::registry::{MappableCommand, TypedBody};

// ── One registry, one dispatcher: lazy-activation dispatch parity ───────────

/// A lazy command's first dispatch leaves identical bookkeeping whether
/// triggered via keypress-style dispatch or the `:` command line — both are
/// an "outer" `Editor::dispatch` call for the same command name, so both
/// stamp dot-repeat/jump/paste bookkeeping identically.
///
/// Not compared against a `call!`-from-another-command path: `call!`'s
/// bookkeeping is deliberately outer-name-wins (see `dispatch.rs`'s
/// `run_steel_command` — "Outer-name-wins: stamp the outer command so `.`
/// replays it, not any inner command the body dispatched via `call!`") — a
/// command reached via an outer wrapper stamps the WRAPPER's name, not the
/// inner command's, so a 3-way keypress/`:`/`call!` identity claim would be
/// asserting behavior the system deliberately does not have.
///
/// Fail oracle: if lazy activation's AFTER-stage bookkeeping (jump/paste/
/// dot-repeat) diverged between the two entry points — e.g. one skipped
/// the repeatable-action stamp — one of the two snapshots would differ.
/// A lazy *mappable* command's first dispatch via keypress activates its
/// plugin and runs the real body — same invariant the typed path exercises
/// in `lazy_typed_command_first_dispatch_via_command_line` below, covering
/// the other half of `declare-plugin`'s two stub kinds
/// (`#:commands`/`#:typed-commands`).
#[test]
fn lazy_command_first_dispatch_via_keypress() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "" (lambda () (call! "delete")) #:repeatable #t)"#,
    );
    let before = snapshot_bookkeeping(&ed);
    ed.execute_keymap_command("bar".into(), Some(1), false);
    let after = snapshot_bookkeeping(&ed);

    assert_ne!(before, after, "first dispatch must run and leave a trace");
    assert_eq!(ed.doc().text().to_string(), "b\n");
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::SteelBacked { .. })
        ),
        "bar stub must have activated on first dispatch"
    );
}

/// `:cmd arg` on a lazy *typed* command's very first dispatch must forward
/// `arg` to the lambda — pins the ordering `Editor::run_typed_steel_command`
/// depends on: activation (which replaces the typed `Lazy` stub with
/// `TypedBody::Steel`) must complete before arg marshalling reads the
/// resolved arity.
///
/// Fail oracle: if activation ran after arg marshalling instead of before,
/// the stub's `Lazy` arity (not yet resolved) would be used instead of the
/// real lambda's arity, and the forwarded arg would never reach `call!`.
#[test]
fn lazy_typed_command_first_dispatch_via_command_line() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:typed-commands '("echo-arg"))"#,
        r#"(define-typed-command! "echo-arg" "" (lambda (x) (when (string? x) (call! x))))"#,
    );
    let before = state(&ed);

    // The typed arg "move-right" is a native command name — echo-arg forwards
    // it straight to `call!`, so a cursor move is observable proof the arg
    // arrived, not just that some command ran.
    type_cmd(&mut ed, ":echo-arg move-right");

    assert_ne!(
        state(&ed),
        before,
        "first :echo-arg dispatch must forward the typed arg (\"move-right\") \
         to the lambda"
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("echo-arg")
                .map(|tc| &tc.body),
            Some(crate::editor::registry::TypedBody::Steel { .. })
        ),
        "echo-arg stub must have activated on first dispatch"
    );
}

/// A failed activation removes EVERY remaining `Lazy` stub of that plugin —
/// not just the one that triggered the activation. Extends
/// `body_error_removes_stub_and_marks_failed` (single-command case) to a
/// plugin declaring two commands, only one of which is dispatched.
///
/// Before `CommandHost::unregister_lazy_stubs_of` was called from
/// `finish_lazy_activation`, a sibling stub survived as a dangling `Lazy`
/// entry pointing at a now-`Failed` plugin until it was itself dispatched
/// (and only then cleaned up by the per-dispatch loop guard) — a behavior
/// improvement this test pins.
///
/// Fail oracle: revert to only unregistering the dispatched stub (the old
/// per-dispatch loop guard alone) → `stub-b` survives as `Lazy` after
/// `stub-a`'s activation fails.
#[test]
fn failed_activation_removes_all_of_the_plugins_stubs_not_just_the_dispatched_one() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:typed-commands '("stub-a" "stub-b"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("stub-a")
                .map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "stub-a must be present before dispatch"
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("stub-b")
                .map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "stub-b must be present before dispatch"
    );

    type_cmd(&mut ed, ":stub-a");

    assert!(
        ed.state.config.registry.get_typed("stub-a").is_none(),
        "the dispatched stub must be gone after failed activation"
    );
    assert!(
        ed.state.config.registry.get_typed("stub-b").is_none(),
        "a sibling stub of the same failed plugin must ALSO be gone \
         immediately — not left dangling until it is itself dispatched"
    );
}

/// `:plugin-status` reports a `Declared` plugin's pending `cmd:` activation
/// entries sourced from the editor's live `Lazy` stubs — the plumbing
/// `lazy_status_string`/`format_status` now require since this crate no
/// longer tracks pending command activations itself.
///
/// Fail oracle: if `typed_plugin_status` stopped passing `registry.lazy_stubs()`
/// through, `:plugin-status` would show no `cmd:` entries for any `Declared`
/// plugin regardless of what it actually declared.
#[test]
fn plugin_status_shows_pending_command_from_live_registry_stubs() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    type_cmd(&mut ed, ":plugin-status");

    assert_eq!(
        ed.doc().display_name(),
        "[plugin-status]",
        ":plugin-status must open the [plugin-status] read-only view"
    );
    let out = ed.doc().text().to_string();
    assert!(
        out.contains("user/tp") && out.contains("cmd:bar"),
        ":plugin-status must show the pending cmd:bar entry for user/tp; got: {out:?}"
    );
}

/// The typed twin of the test above — a pending `#:typed-commands` entry
/// must display as `:cmd:` (`:`-only, never key-bindable), not the bare
/// `cmd:` a mappable entry gets, so the user knows which reachability a
/// still-`Declared` plugin's pending name will have.
///
/// Flip: without the kind split, this would show `cmd:bar` instead — the
/// same string `plugin_status_shows_pending_command_from_live_registry_stubs`
/// asserts for a *mappable* pending entry.
#[test]
fn plugin_status_shows_pending_typed_command_with_kind_tag() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    type_cmd(&mut ed, ":plugin-status");

    let out = ed.doc().text().to_string();
    assert!(
        out.contains("user/tp") && out.contains(":cmd:bar"),
        ":plugin-status must show the pending :cmd:bar entry for user/tp; got: {out:?}"
    );
    assert!(
        !out.contains(" cmd:bar") && !out.contains(",cmd:bar"),
        "a typed-only pending entry must not also appear as a bare cmd:; got: {out:?}"
    );
}

// ── runtime/init.scm.example coverage ────────────────────────────────────────

/// Path to the shipped example config, the same file a fresh install's
/// `:help`/README points users to copy to `~/.config/hume/init.scm`.
const INIT_SCM_EXAMPLE_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime/init.scm.example");

/// The real, shipped `runtime/init.scm.example` evaluates cleanly end to end
/// against the real runtime — a removed builtin or a renamed keyword arg
/// (exactly this commit's kind of change) fails here, not first when a user
/// copies the example and starts HUME.
///
/// Fail oracle: rename a keyword arg in `runtime/init.scm.example` (e.g.
/// `#:events` to `#:event`) → `eval_init` fails and this test catches it.
#[test]
fn init_scm_example_is_valid_source() {
    let example =
        std::fs::read_to_string(INIT_SCM_EXAMPLE_PATH).expect("init.scm.example must be readable");
    let tmp = safe_tempdir();
    let guard = RealRuntimeGuard::new();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let mut host = ScriptingHost::new();
    eval_with_real_host(&mut ed, &mut host, &example, tmp.path());
    drop(guard);
}
