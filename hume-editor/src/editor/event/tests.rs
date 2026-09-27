use hume_engine::pipeline::{BufferId, PaneId};
use hume_scripting::PaneHandle;
use steel::rvals::SteelVal;

use super::*;

/// One sample per variant — a `const` slice is impossible once variants
/// carry `String`/`serde_json::Value` payloads, unlike the old fieldless
/// enum. Field values are distinctive (not defaults) so the shape tests
/// below can tell fields apart if `steel_args` ever swaps two of them.
fn all_variants() -> Vec<EditorEvent> {
    let buffer = BufferId::default();
    let target = PaneHandle::with_pane(buffer, sample_pane_id());
    vec![
        EditorEvent::OnBufferOpen { buffer },
        EditorEvent::OnBufferClose { buffer },
        EditorEvent::OnBufferSave { buffer },
        EditorEvent::OnBufferEnter { target },
        EditorEvent::OnFocusGained,
        EditorEvent::OnModeChange {
            from: Mode::Insert,
            to: Mode::Normal,
        },
        EditorEvent::OnLanguageSet {
            buffer,
            language: Some("rust".to_string()),
        },
        EditorEvent::OnLspAttach {
            buffer,
            server: "rust-analyzer".to_string(),
        },
        EditorEvent::OnLspDetach {
            buffer,
            server: "rust-analyzer".to_string(),
        },
        EditorEvent::OnDiagnosticsChanged { buffer },
        EditorEvent::OnViewportChange {
            target,
            first_line: hume_rope::line::ContentLine::new(3),
            end_line: hume_rope::line::ContentLine::new(42),
        },
        EditorEvent::OnTriggerChar {
            target,
            ch: '.',
            source: "lsp".to_string(),
        },
        EditorEvent::OnCompletionAccept {
            target,
            item: hume_scripting::json::JsonHandle::new(serde_json::json!({"label": "foo"})),
        },
        EditorEvent::OnOptionChange {
            key: "lsp.inlay-hints".to_string(),
            value: "true".to_string(),
        },
        EditorEvent::OnTextChanged { buffer },
    ]
}

/// A distinctive, non-default `PaneId` for the pane-carrying variants — a
/// fresh slotmap allocation, not `PaneId::default()`, so a test comparing
/// against `PaneHandle::buffer_only`'s `None` pane can't pass by accident.
fn sample_pane_id() -> PaneId {
    let mut sm: slotmap::SlotMap<PaneId, ()> = slotmap::SlotMap::with_key();
    sm.insert(())
}

/// Every variant has a name, that name is in `EVENT_NAMES`, and the two
/// lists are the same length with no duplicates.
#[test]
fn every_variant_has_a_name_and_matches_the_known_names_table() {
    let variants = all_variants();
    let names: Vec<&str> = variants.iter().map(|e| e.name()).collect();

    for name in &names {
        assert!(
            EVENT_NAMES.contains(name),
            "{name} is returned by name() but missing from EVENT_NAMES"
        );
    }

    assert_eq!(
        EVENT_NAMES.len(),
        variants.len(),
        "EVENT_NAMES and the variant list have drifted apart in length"
    );

    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "two variants share the same Steel name"
    );
}

// ── steel_args shape table ──────────────────────────────────────────────────
//
// Expected values are written by hand from the documented Steel contract,
// never derived from `steel_args` itself. Swapping or dropping a field in a
// `steel_args` match arm makes the matching row fail.

/// `SteelPane` doesn't expose its inner `PaneHandle` outside
/// `hume-scripting` — compare the wrapped `SteelVal` for equality against a
/// freshly wrapped `expected` instead of unwrapping.
fn assert_steel_pane(args: &[SteelVal], idx: usize, expected: PaneHandle) {
    assert_eq!(
        args[idx],
        hume_scripting::SteelPane::new(expected).into_steel_val()
    );
}

fn steel_string(args: &[SteelVal], idx: usize) -> String {
    match &args[idx] {
        SteelVal::StringV(s) => s.to_string(),
        other => panic!("expected a StringV, got {other:?}"),
    }
}

#[test]
fn buffer_only_events_carry_one_pane_less_handle_arg() {
    let buffer = BufferId::default();
    for event in [
        EditorEvent::OnBufferOpen { buffer },
        EditorEvent::OnBufferClose { buffer },
        EditorEvent::OnBufferSave { buffer },
        EditorEvent::OnDiagnosticsChanged { buffer },
        EditorEvent::OnTextChanged { buffer },
    ] {
        let args = event.steel_args();
        assert_eq!(args.len(), 1, "{event:?} must carry exactly one arg");
        assert_steel_pane(&args, 0, PaneHandle::buffer_only(buffer));
    }
}

/// `OnBufferEnter` carries a pane too — always `Some`, since a buffer only
/// "enters" by way of some pane showing it (unlike the buffer-only events
/// above, which have no pane of their own to name).
#[test]
fn on_buffer_enter_carries_its_pane() {
    let buffer = BufferId::default();
    let pane = sample_pane_id();
    let target = PaneHandle::with_pane(buffer, pane);
    let event = EditorEvent::OnBufferEnter { target };
    let args = event.steel_args();
    assert_eq!(args.len(), 1);
    assert_steel_pane(&args, 0, target);
}

#[test]
fn on_focus_gained_carries_no_args() {
    assert_eq!(
        EditorEvent::OnFocusGained.steel_args().len(),
        0,
        "on-focus-gained is payload-free — it sweeps every buffer, not one"
    );
}

#[test]
fn on_mode_change_stringifies_in_steel_args_not_at_the_raise_site() {
    let event = EditorEvent::OnModeChange {
        from: Mode::Insert,
        to: Mode::Normal,
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 2);
    assert_eq!(steel_string(&args, 0), "insert");
    assert_eq!(steel_string(&args, 1), "normal");
}

#[test]
fn on_language_set_carries_buffer_and_language_name() {
    let buffer = BufferId::default();
    let event = EditorEvent::OnLanguageSet {
        buffer,
        language: Some("python".to_string()),
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 2);
    assert_steel_pane(&args, 0, PaneHandle::buffer_only(buffer));
    assert_eq!(steel_string(&args, 1), "python");
}

/// The only sentinel arg in the whole event set: no language is `#f`, not
/// an empty string or an omitted arg.
#[test]
fn on_language_set_with_no_language_sends_false() {
    let event = EditorEvent::OnLanguageSet {
        buffer: BufferId::default(),
        language: None,
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 2);
    assert!(matches!(args[1], SteelVal::BoolV(false)));
}

#[test]
fn on_lsp_attach_and_detach_carry_buffer_and_server_name() {
    let buffer = BufferId::default();
    for event in [
        EditorEvent::OnLspAttach {
            buffer,
            server: "rust-analyzer".to_string(),
        },
        EditorEvent::OnLspDetach {
            buffer,
            server: "rust-analyzer".to_string(),
        },
    ] {
        let args = event.steel_args();
        assert_eq!(args.len(), 2);
        assert_steel_pane(&args, 0, PaneHandle::buffer_only(buffer));
        assert_eq!(steel_string(&args, 1), "rust-analyzer");
    }
}

#[test]
fn on_viewport_change_carries_pane_and_both_line_bounds() {
    let buffer = BufferId::default();
    let pane = sample_pane_id();
    let target = PaneHandle::with_pane(buffer, pane);
    let event = EditorEvent::OnViewportChange {
        target,
        first_line: hume_rope::line::ContentLine::new(3),
        end_line: hume_rope::line::ContentLine::new(42),
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 3);
    assert_steel_pane(&args, 0, target);
    assert!(matches!(args[1], SteelVal::IntV(3)));
    assert!(matches!(args[2], SteelVal::IntV(42)));
}

/// `char` renders as a 1-char Steel *string*, not a Steel char — pins the
/// documented `on-trigger-char` contract.
#[test]
fn on_trigger_char_sends_char_as_a_one_char_string() {
    let buffer = BufferId::default();
    let pane = sample_pane_id();
    let target = PaneHandle::with_pane(buffer, pane);
    let event = EditorEvent::OnTriggerChar {
        target,
        ch: '.',
        source: "lsp".to_string(),
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 3);
    assert_steel_pane(&args, 0, target);
    assert_eq!(steel_string(&args, 1), ".");
    assert_eq!(steel_string(&args, 2), "lsp");
}

/// `OnCompletionAccept`'s item crosses as an opaque `JsonHandle` onto the
/// original JSON, not a hand-rolled conversion or a decoded hashmap.
#[test]
fn on_completion_accept_item_crosses_as_a_json_handle() {
    let buffer = BufferId::default();
    let pane = sample_pane_id();
    let target = PaneHandle::with_pane(buffer, pane);
    let item = serde_json::json!({"label": "foo", "kind": 3});
    let event = EditorEvent::OnCompletionAccept {
        target,
        item: hume_scripting::json::JsonHandle::new(item.clone()),
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 2);
    assert_steel_pane(&args, 0, target);
    let handle = hume_scripting::json::downcast_json_handle(&args[1])
        .expect("item must cross as a JsonHandle");
    assert_eq!(handle.value(), &item);
}

#[test]
fn on_option_change_carries_key_and_value_no_buffer_id() {
    let event = EditorEvent::OnOptionChange {
        key: "lsp.inlay-hints".to_string(),
        value: "true".to_string(),
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 2, "payload is (key value) — no buffer id");
    assert_eq!(steel_string(&args, 0), "lsp.inlay-hints");
    assert_eq!(steel_string(&args, 1), "true");
}
