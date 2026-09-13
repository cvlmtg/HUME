//! `bind-key-extend!`, `unbind-key!`, and prelude macro behavior tests.

use super::*;

// ── bind-key-extend! ──────────────────────────────────────────────────────

#[test]
fn bind_key_extend_queues_force_extend_effect() {
    let mut h = host();
    let mut mock = MockHost::new();

    let effects = h
        .eval_source(r#"(bind-key-extend! 'normal "z" "select-line")"#, &mut mock)
        .unwrap();
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::BindKey { cmd, force_extend: true, .. }] if cmd == "select-line"
        ),
        "bind-key-extend! must produce force_extend = true; got: {effects:?}"
    );
}

#[test]
fn bind_key_does_not_force_extend() {
    let mut h = host();
    let mut mock = MockHost::new();

    let effects = h
        .eval_source(r#"(bind-key! 'normal "z" "select-line")"#, &mut mock)
        .unwrap();
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::BindKey {
                force_extend: false,
                ..
            }]
        ),
        "bind-key! must produce force_extend = false; got: {effects:?}"
    );
}

#[test]
fn bind_key_extend_invalid_mode_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source(r#"(bind-key-extend! 'visual "f" "cmd")"#, &mut mock)
        .unwrap_err();
    assert!(err.contains("mode"), "got: {err}");
}

// ── unbind-key! ───────────────────────────────────────────────────────────

#[test]
fn unbind_key_queues_unbind_effect() {
    let mut h = host();
    let mut mock = MockHost::new();

    let effects = h
        .eval_source(r#"(unbind-key! 'normal "h")"#, &mut mock)
        .unwrap();

    // Whether 'h' was bound is `Keymap`'s business, not the builtin's —
    // `remove_sequence_nonexistent_is_noop` (editor/keymap/mod.rs) owns that.
    use termina::event::{KeyCode, KeyEvent, Modifiers};
    let h_key = KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::UnbindKey { mode: BindMode::Normal, keys }] if keys.as_slice() == [h_key]
        ),
        "expected one Effect::UnbindKey for 'h'; got: {effects:?}"
    );
}

#[test]
fn unbind_key_invalid_mode_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source(r#"(unbind-key! 'visual "h")"#, &mut mock)
        .unwrap_err();
    assert!(err.contains("mode"), "got: {err}");
}

// ── Prelude macro behavior ────────────────────────────────────────────────────
//
// The prelude defines bind-keys!, bind-keys-extend!, and unbind-keys! as
// syntax-rules batch wrappers over the underlying single-pair builtins.
// These tests load the three macros via eval_source (as init_scripting does via
// eval_init before init.scm), then exercise each macro.
//
// Independent oracle: the expected bindings come from the literal key/cmd pairs
// passed to the macro — not from re-reading the keymap.
// Zero-effect check: swap a cmd name in a pair; the assertion catches it because
// it compares against the literal name from the input, not "whatever the keymap says".

/// Real `runtime/scheme/prelude.scm` source, read fresh per call so these tests
/// exercise the file plugin authors actually get — same `CARGO_MANIFEST_DIR`-
/// relative approach as `editor::tests::scripting_grammar::runtime_scheme_dir`,
/// which is independent of the test runner's CWD.
fn real_prelude_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime/scheme/prelude.scm");
    std::fs::read_to_string(&path).unwrap()
}

#[test]
fn prelude_bind_keys_batch_binds_multiple() {
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(&real_prelude_source(), &mut mock).unwrap();
    let effects = h
        .eval_source(
            r#"(bind-keys! 'normal
             ("z z" "move-left")
             ("z l" "move-right"))"#,
            &mut mock,
        )
        .unwrap();

    let z = KeyEvent::new(KeyCode::Char('z'), Modifiers::NONE);
    let l = KeyEvent::new(KeyCode::Char('l'), Modifiers::NONE);

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::BindKey { keys: k1, cmd: c1, force_extend: false, .. },
                Effect::BindKey { keys: k2, cmd: c2, force_extend: false, .. },
            ] if k1.as_slice() == [z, z] && c1 == "move-left"
                && k2.as_slice() == [z, l] && c2 == "move-right"
        ),
        "bind-keys! must expand to one non-force-extend bind per pair, in order; \
         got: {effects:?}"
    );
}

#[test]
fn prelude_bind_keys_extend_creates_force_extend_leaves() {
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(&real_prelude_source(), &mut mock).unwrap();
    let effects = h
        .eval_source(
            r#"(bind-keys-extend! 'normal
             ("Q" "select-line")
             ("W" "select-to-end"))"#,
            &mut mock,
        )
        .unwrap();

    let q = KeyEvent::new(KeyCode::Char('Q'), Modifiers::NONE);
    let w = KeyEvent::new(KeyCode::Char('W'), Modifiers::NONE);

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::BindKey { keys: k1, cmd: c1, force_extend: true, .. },
                Effect::BindKey { keys: k2, cmd: c2, force_extend: true, .. },
            ] if k1.as_slice() == [q] && c1 == "select-line"
                && k2.as_slice() == [w] && c2 == "select-to-end"
        ),
        "bind-keys-extend! must expand to force-extending binds, in order; got: {effects:?}"
    );
}

#[test]
fn prelude_unbind_keys_batch_removes_bindings() {
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(&real_prelude_source(), &mut mock).unwrap();
    let effects = h
        .eval_source(r#"(unbind-keys! 'normal "h" "l")"#, &mut mock)
        .unwrap();

    let h_key = KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE);
    let l_key = KeyEvent::new(KeyCode::Char('l'), Modifiers::NONE);

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::UnbindKey { keys: k1, .. },
                Effect::UnbindKey { keys: k2, .. },
            ] if k1.as_slice() == [h_key] && k2.as_slice() == [l_key]
        ),
        "unbind-keys! must expand to one unbind per key, in order; got: {effects:?}"
    );
}

/// Verify the prelude eval_init → init.scm eval_init sequence: prelude macros
/// defined by the first eval_init are available in the second.
#[test]
fn prelude_eval_init_sequence_makes_macros_available_to_init_scm() {
    use std::io::Write as _;

    let mut h = host();
    let mut mock = MockHost::new();

    let builtin_names: rustc_hash::FxHashSet<String> = Default::default();

    // Stage a prelude file and an init.scm that uses its macros.
    let dir = tempfile::tempdir().unwrap();
    let prelude_path = dir.path().join("prelude.scm");
    let init_path = dir.path().join("init.scm");

    std::fs::write(&prelude_path, real_prelude_source()).unwrap();
    let mut f = std::fs::File::create(&init_path).unwrap();
    writeln!(
        f,
        r#"(bind-keys! 'normal ("Q Q" "move-left") ("Q W" "move-right"))"#
    )
    .unwrap();

    // Load prelude first, then init.scm — mirroring init_scripting's sequence.
    h.eval_init(&prelude_path, 10_000, &mut mock, builtin_names.clone())
        .expect("prelude eval_init must succeed");
    assert!(
        mock.registered_cmds.is_empty(),
        "prelude must define no commands; got {:?}",
        mock.registered_cmds
            .iter()
            .map(|d| &d.name)
            .collect::<Vec<_>>()
    );

    let effects = h
        .eval_init(&init_path, 10_000, &mut mock, builtin_names)
        .expect("init.scm using bind-keys! must succeed after prelude is loaded");

    use termina::event::{KeyCode, KeyEvent, Modifiers};
    let q = KeyEvent::new(KeyCode::Char('Q'), Modifiers::NONE);
    let w = KeyEvent::new(KeyCode::Char('W'), Modifiers::NONE);

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::BindKey { keys: k1, cmd: c1, .. },
                Effect::BindKey { keys: k2, cmd: c2, .. },
            ] if k1.as_slice() == [q, q] && c1 == "move-left"
                && k2.as_slice() == [q, w] && c2 == "move-right"
        ),
        "init.scm's bind-keys! must expand via the prelude macro; got: {effects:?}"
    );
}

/// When init.scm uses bind-keys! but the prelude was never loaded, the eval
/// fails with a clear error (macro undefined) — not a panic.
#[test]
fn bind_keys_without_prelude_fails_gracefully() {
    let mut h = host();
    let mut mock = MockHost::new();

    // bind-keys! is NOT defined — init.scm uses it directly.
    let err = h
        .eval_source(r#"(bind-keys! 'normal ("z" "move-left"))"#, &mut mock)
        .unwrap_err();

    // Steel reports an unbound identifier or similar error; the editor survives.
    assert!(
        !err.is_empty(),
        "using bind-keys! without prelude must return an error"
    );
}
