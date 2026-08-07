use hume_engine::pipeline::BufferId;
use steel::rvals::SteelVal;

use super::*;

/// One sample per variant — a `const` slice is impossible once variants
/// carry `String`/`serde_json::Value` payloads, unlike the old fieldless
/// enum. Field values are distinctive (not defaults) so the shape tests
/// below can tell fields apart if `steel_args` ever swaps two of them.
fn all_variants() -> Vec<EditorEvent> {
    let buffer = BufferId::default();
    vec![
        EditorEvent::OnBufferOpen { buffer },
        EditorEvent::OnBufferClose { buffer },
        EditorEvent::OnBufferSave { buffer },
        EditorEvent::OnBufferEnter { buffer },
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
            buffer,
            first_line: 3,
            last_line: 42,
        },
        EditorEvent::OnTriggerChar {
            buffer,
            ch: '.',
            source: "lsp".to_string(),
        },
        EditorEvent::OnCompletionAccept {
            buffer,
            item: serde_json::json!({"label": "foo"}),
        },
        EditorEvent::OnCompletionRefilter {
            buffer,
            filter_text: "fo".to_string(),
        },
        EditorEvent::OnOptionChange {
            key: "lsp.inlay-hints".to_string(),
            value: "true".to_string(),
        },
        EditorEvent::OnTextChanged { buffer },
    ]
}

/// Every variant has a name, that name is in `EVENT_NAMES`, and the two
/// lists are the same length with no duplicates.
///
/// Fail oracle: drop a `name()` match arm's string, or remove it from
/// `EVENT_NAMES`, or duplicate an entry in `EVENT_NAMES` — any of those
/// fails one of the three assertions below.
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
// never derived from `steel_args` itself — an independent oracle. Fail
// oracle for each: swap two fields in the corresponding `steel_args` match
// arm, or drop one, and the row fails.

/// `SteelBufferId` doesn't expose its inner `BufferId` outside
/// `hume-scripting` — compare the wrapped `SteelVal` for equality against a
/// freshly wrapped `expected` instead of unwrapping.
fn assert_steel_buffer_id(args: &[SteelVal], idx: usize, expected: BufferId) {
    assert_eq!(
        args[idx],
        hume_scripting::SteelBufferId::new(expected).into_steel_val()
    );
}

fn steel_string(args: &[SteelVal], idx: usize) -> String {
    match &args[idx] {
        SteelVal::StringV(s) => s.to_string(),
        other => panic!("expected a StringV, got {other:?}"),
    }
}

#[test]
fn buffer_only_events_carry_one_buffer_id_arg() {
    let buffer = BufferId::default();
    for event in [
        EditorEvent::OnBufferOpen { buffer },
        EditorEvent::OnBufferClose { buffer },
        EditorEvent::OnBufferSave { buffer },
        EditorEvent::OnBufferEnter { buffer },
        EditorEvent::OnDiagnosticsChanged { buffer },
        EditorEvent::OnTextChanged { buffer },
    ] {
        let args = event.steel_args();
        assert_eq!(args.len(), 1, "{event:?} must carry exactly one arg");
        assert_steel_buffer_id(&args, 0, buffer);
    }
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
    assert_steel_buffer_id(&args, 0, buffer);
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
        assert_steel_buffer_id(&args, 0, buffer);
        assert_eq!(steel_string(&args, 1), "rust-analyzer");
    }
}

#[test]
fn on_viewport_change_carries_buffer_and_both_line_bounds() {
    let buffer = BufferId::default();
    let event = EditorEvent::OnViewportChange {
        buffer,
        first_line: 3,
        last_line: 42,
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 3);
    assert_steel_buffer_id(&args, 0, buffer);
    assert!(matches!(args[1], SteelVal::IntV(3)));
    assert!(matches!(args[2], SteelVal::IntV(42)));
}

/// `char` renders as a 1-char Steel *string*, not a Steel char — pins the
/// documented `on-trigger-char` contract.
#[test]
fn on_trigger_char_sends_char_as_a_one_char_string() {
    let buffer = BufferId::default();
    let event = EditorEvent::OnTriggerChar {
        buffer,
        ch: '.',
        source: "lsp".to_string(),
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 3);
    assert_steel_buffer_id(&args, 0, buffer);
    assert_eq!(steel_string(&args, 1), ".");
    assert_eq!(steel_string(&args, 2), "lsp");
}

/// `OnCompletionAccept`'s item JSON round-trips through `json_to_steel`,
/// not a hand-rolled conversion left over at the raise site.
#[test]
fn on_completion_accept_json_round_trips_through_json_to_steel() {
    let buffer = BufferId::default();
    let item = serde_json::json!({"label": "foo", "kind": 3});
    let event = EditorEvent::OnCompletionAccept {
        buffer,
        item: item.clone(),
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 2);
    assert_steel_buffer_id(&args, 0, buffer);
    assert_eq!(args[1], hume_scripting::json::json_to_steel(&item));
}

#[test]
fn on_completion_refilter_carries_buffer_and_filter_text() {
    let buffer = BufferId::default();
    let event = EditorEvent::OnCompletionRefilter {
        buffer,
        filter_text: "fo".to_string(),
    };
    let args = event.steel_args();
    assert_eq!(args.len(), 2);
    assert_steel_buffer_id(&args, 0, buffer);
    assert_eq!(steel_string(&args, 1), "fo");
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
