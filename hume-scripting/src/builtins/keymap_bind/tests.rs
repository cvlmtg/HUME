use super::*;
use crate::host::KeymapTarget;
use crate::test_support::SteelCtxTestHarness;

/// Effects queued so far, in emission order.
fn effects(h: &SteelCtxTestHarness) -> Vec<&Effect> {
    h.effects.iter().map(|e| &e.effect).collect()
}

// ── Global binds are init-only, named keymaps are not ─────────────────────

fn sym(name: &str) -> SteelVal {
    SteelVal::SymbolV(name.into())
}

fn no_keymap() -> SteelVal {
    SteelVal::BoolV(false)
}

/// A global `bind-key!`, `bind-key-extend!` or `unbind-key!` raises in a
/// command body (`EvalMode::Command`) and queues nothing.
#[test]
fn global_binds_are_rejected_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let results = {
        let mut ctx = h.ctx();
        [
            bind_key(
                &mut ctx,
                sym("normal"),
                "z".into(),
                "move-right".into(),
                no_keymap(),
            ),
            bind_key_extend(
                &mut ctx,
                sym("normal"),
                "z".into(),
                "move-right".into(),
                no_keymap(),
            ),
            unbind_key(&mut ctx, sym("normal"), "z".into(), no_keymap()),
        ]
    };
    for result in results {
        let msg = result
            .expect_err("a global bind must raise in command mode")
            .to_string();
        assert!(
            msg.contains("init"),
            "error must mention 'init'; got: {msg}"
        );
    }
    assert!(effects(&h).is_empty());
}

/// With `#:keymap`, `bind-key!` works in a command body and queues a bind
/// into that named keymap.
#[test]
fn bind_key_with_a_keymap_queues_a_named_bind_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        bind_key(
            &mut ctx,
            sym("normal"),
            "z".into(),
            "move-right".into(),
            sym("md-keys"),
        )
        .expect("a named bind is allowed in command mode");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::BindKey { cmd, force_extend: false, keymap: KeymapTarget::Named(name), .. }]
                if cmd == "move-right" && name == "md-keys"
        ),
        "got: {:?}",
        effects(&h)
    );
}

#[test]
fn bind_key_extend_with_a_keymap_queues_a_named_bind_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        bind_key_extend(
            &mut ctx,
            sym("normal"),
            "z".into(),
            "select-line".into(),
            sym("md-keys"),
        )
        .expect("a named bind is allowed in command mode");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::BindKey { force_extend: true, keymap: KeymapTarget::Named(name), .. }]
                if name == "md-keys"
        ),
        "got: {:?}",
        effects(&h)
    );
}

#[test]
fn unbind_key_with_a_keymap_queues_a_named_unbind_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        unbind_key(&mut ctx, sym("normal"), "z".into(), sym("md-keys"))
            .expect("a named unbind is allowed in command mode");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::UnbindKey { keymap: KeymapTarget::Named(name), .. }] if name == "md-keys"
        ),
        "got: {:?}",
        effects(&h)
    );
}

/// A keymap is named by a symbol; a string raises and queues nothing.
#[test]
fn a_keymap_named_by_a_string_is_rejected() {
    let mut h = SteelCtxTestHarness::new();
    let result = {
        let mut ctx = h.ctx();
        bind_key(
            &mut ctx,
            sym("normal"),
            "z".into(),
            "move-right".into(),
            SteelVal::StringV("md-keys".into()),
        )
    };
    let msg = result
        .expect_err("a string keymap name must raise")
        .to_string();
    assert!(msg.contains("#:keymap"), "got: {msg}");
    assert!(effects(&h).is_empty());
}

// ── Mode validation ───────────────────────────────────────────────────────

/// `bind-key!` rejects an unknown mode name.
///
/// Without `mode_from_symbol` validation, 'visual would fall into an
/// arbitrary match arm and insert into the wrong trie.
#[test]
fn bind_key_invalid_mode_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = bind_key(
        &mut ctx,
        SteelVal::SymbolV("visual".into()),
        "z".into(),
        "move-right".into(),
        no_keymap(),
    );
    assert!(result.is_err(), "bind-key! must reject unknown mode");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("mode"),
        "error must mention 'mode'; got: {msg}"
    );
    drop(ctx);
    assert!(
        effects(&h).is_empty(),
        "validation must reject before queueing anything"
    );
}

/// `unbind-key!` rejects an unknown mode name.
#[test]
fn unbind_key_invalid_mode_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = unbind_key(
        &mut ctx,
        SteelVal::SymbolV("visual".into()),
        "z".into(),
        no_keymap(),
    );
    assert!(result.is_err(), "unbind-key! must reject unknown mode");
    drop(ctx);
    assert!(
        effects(&h).is_empty(),
        "validation must reject before queueing anything"
    );
}

/// `bind-key!` rejects a string mode: mode must be a symbol.
#[test]
fn bind_key_string_mode_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = bind_key(
        &mut ctx,
        SteelVal::StringV("normal".into()),
        "z".into(),
        "move-right".into(),
        no_keymap(),
    );
    assert!(result.is_err(), "bind-key! must reject a string mode");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("symbol"),
        "error must mention 'symbol'; got: {msg}"
    );
}

// ── Key-sequence parsing ──────────────────────────────────────────────────

/// `bind-key!` rejects an invalid key-sequence string.
///
/// Parsing that returned `Ok([])` on bad input would insert the binding
/// under an empty key, where no keypress can reach it.
#[test]
fn bind_key_invalid_key_sequence_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    // "<NOTAKEY>" is not a valid key name.
    let result = bind_key(
        &mut ctx,
        SteelVal::SymbolV("normal".into()),
        "<NOTAKEY>".into(),
        "move-right".into(),
        no_keymap(),
    );
    assert!(
        result.is_err(),
        "bind-key! must reject invalid key sequences"
    );
    drop(ctx);
    assert!(
        effects(&h).is_empty(),
        "validation must reject before queueing anything"
    );
}

/// `unbind-key!` also validates the key sequence.
#[test]
fn unbind_key_invalid_key_sequence_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = unbind_key(
        &mut ctx,
        SteelVal::SymbolV("normal".into()),
        "<NOTAKEY>".into(),
        no_keymap(),
    );
    assert!(
        result.is_err(),
        "unbind-key! must reject invalid key sequences"
    );
    drop(ctx);
    assert!(
        effects(&h).is_empty(),
        "validation must reject before queueing anything"
    );
}

// ── Guard passes, effect queued ───────────────────────────────────────────

/// In init mode with valid args, `bind-key!` passes the guard and queues an
/// `Effect::BindKey` carrying the parsed mode/keys/command. Nothing touches
/// the keymap at builtin time; the editor applies it later via
/// `Editor::apply_script_effects`.
#[test]
fn bind_key_init_mode_queues_bind_effect() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = bind_key(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "z".into(),
            "move-right".into(),
            no_keymap(),
        );
        assert!(result.is_ok(), "bind-key! must succeed; got: {result:?}");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::BindKey { mode: BindMode::Normal, keys, cmd, force_extend: false, keymap: KeymapTarget::Global }]
                if keys.len() == 1 && cmd == "move-right"
        ),
        "expected one Effect::BindKey for 'z' → move-right; got: {:?}",
        effects(&h)
    );
}

/// `bind-key-extend!` queues the same effect with `force_extend: true`;
/// the flag is the only thing distinguishing it from `bind-key!`.
#[test]
fn bind_key_extend_queues_force_extend_effect() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        bind_key_extend(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "z".into(),
            "select-line".into(),
            no_keymap(),
        )
        .expect("bind-key-extend! must succeed");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::BindKey { force_extend: true, cmd, .. }] if cmd == "select-line"
        ),
        "expected Effect::BindKey with force_extend: true; got: {:?}",
        effects(&h)
    );
}

/// `'command` binds a key in the command mode prompt.
#[test]
fn bind_key_command_mode_queues_bind_effect() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        bind_key(
            &mut ctx,
            SteelVal::SymbolV("command".into()),
            "ctrl-r".into(),
            "my-cmd".into(),
            no_keymap(),
        )
        .expect("bind-key! 'command must succeed");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::BindKey { mode: BindMode::Command, keys, cmd, force_extend: false, keymap: KeymapTarget::Global }]
                if keys.len() == 1 && cmd == "my-cmd"
        ),
        "expected one Effect::BindKey in command mode; got: {:?}",
        effects(&h)
    );
}

/// Insert and command mode dispatch one key at a time, so a sequence
/// bound there could never fire.
#[test]
fn bind_key_rejects_a_key_sequence_in_insert_and_command_modes() {
    for mode in ["insert", "command"] {
        let mut h = SteelCtxTestHarness::new();
        let msg = {
            let mut ctx = h.ctx_init();
            bind_key(
                &mut ctx,
                SteelVal::SymbolV(mode.into()),
                "ctrl-x a".into(),
                "my-cmd".into(),
                no_keymap(),
            )
            .unwrap_err()
            .to_string()
        };
        assert!(msg.contains("single key"), "{mode}: got: {msg}");
        assert!(msg.contains(mode), "{mode}: got: {msg}");
        assert!(effects(&h).is_empty(), "{mode}: nothing queued");
    }
}

/// Only a Normal-mode binding reads the always-extend flag.
#[test]
fn bind_key_extend_rejects_every_mode_but_normal() {
    for mode in ["extend", "insert", "command"] {
        let mut h = SteelCtxTestHarness::new();
        let msg = {
            let mut ctx = h.ctx_init();
            bind_key_extend(
                &mut ctx,
                SteelVal::SymbolV(mode.into()),
                "z".into(),
                "my-cmd".into(),
                no_keymap(),
            )
            .unwrap_err()
            .to_string()
        };
        assert!(msg.contains("'normal"), "{mode}: got: {msg}");
        assert!(effects(&h).is_empty(), "{mode}: nothing queued");
    }
}

/// `unbind-key!` queues `Effect::UnbindKey`, deferred like the two
/// binders so a same-eval bind-then-unbind on one key applies in Steel's
/// emission order rather than the unbind racing ahead.
#[test]
fn unbind_key_queues_unbind_effect() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        unbind_key(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "z".into(),
            no_keymap(),
        )
        .expect("unbind-key! must succeed");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::UnbindKey { mode: BindMode::Normal, keys, keymap: KeymapTarget::Global }] if keys.len() == 1
        ),
        "expected Effect::UnbindKey; got: {:?}",
        effects(&h)
    );
}

// ── Plugin-load context (plugin_stack non-empty) ──────────────────────────

/// When `plugin_stack` is non-empty (inside a plugin body), all four bind
/// builtins are permitted even in `EvalMode::PluginActivation`.
#[test]
fn bind_key_permitted_during_plugin_load() {
    use crate::attribution::{EntryId, PluginId};
    let mut h = SteelCtxTestHarness::new();
    h.plugin_stack
        .push(EntryId::main(PluginId::parse("core:myplugin").unwrap()));
    {
        let mut ctx = h.ctx(); // EvalMode::PluginActivation → allowed
        let result = bind_key(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "z".into(),
            "cmd".into(),
            no_keymap(),
        );
        assert!(
            result.is_ok(),
            "bind-key! must not hit the init guard during plugin load; got: {result:?}"
        );
    }
    assert!(
        matches!(effects(&h).as_slice(), [Effect::BindKey { .. }]),
        "expected the bind to be queued; got: {:?}",
        effects(&h)
    );
}

// ── attach-keymap! / detach-keymap! ─────────────────────────────────────────

#[test]
fn attach_keymap_queues_an_attach_for_the_panes_buffer() {
    use crate::test_support::{default_bid, default_pane};
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        attach_keymap(&mut ctx, default_pane(), sym("md-keys")).expect("attach must succeed");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::AttachKeymap { buffer, name }] if *buffer == default_bid() && name == "md-keys"
        ),
        "got: {:?}",
        effects(&h)
    );
}

#[test]
fn detach_keymap_queues_a_detach_for_the_panes_buffer() {
    use crate::test_support::{default_bid, default_pane};
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        detach_keymap(&mut ctx, default_pane(), sym("md-keys")).expect("detach must succeed");
    }
    assert!(
        matches!(
            effects(&h).as_slice(),
            [Effect::DetachKeymap { buffer, name }] if *buffer == default_bid() && name == "md-keys"
        ),
        "got: {:?}",
        effects(&h)
    );
}

#[test]
fn attach_keymap_takes_a_symbol_name() {
    use crate::test_support::default_pane;
    let mut h = SteelCtxTestHarness::new();
    let result = {
        let mut ctx = h.ctx();
        attach_keymap(
            &mut ctx,
            default_pane(),
            SteelVal::StringV("md-keys".into()),
        )
    };
    let msg = result.expect_err("a string name must raise").to_string();
    assert!(msg.contains("attach-keymap!"), "got: {msg}");
    assert!(effects(&h).is_empty());
}
