use super::*;
use crate::test_support::SteelCtxTestHarness;

// ── Gate (init mode rejection) ────────────────────────────────────────────
//
// Every builtin below is `cmd`-gated in `builtins!`'s registration table:
// the gate lives in the registration wrapper closure, not the function
// body, so these test the gate primitive directly rather than calling the
// builtin (its body has no guard to hit).

// `char-offset->line`/`line->char-offset`'s wrong-type-argument checks are covered
// centrally, by `args::tests`' own unit tests on the `Usize` `FromSteelVal`
// newtype their `builtins!` table entries declare. That decode happens at
// Steel's own registration boundary, before either function's body (which
// takes a plain `usize`) ever runs, so it cannot be exercised by calling the
// function directly with a malformed `SteelVal`.
//
// Likewise, every explicit-`pane` builtin's pane check (an open buffer, a
// pane that shows it, focus) runs in its pane argument type's
// `BuiltinArg::resolve`, in the `builtins!`-registered closure, before the
// function body ever runs. A direct call here has no way to reach that
// check at all; it's covered once, centrally, through a real
// `ScriptingHost` by the `builtins::tests::pane_builtins_*` tests.

// ── Command-mode success paths (NullHost read methods return None/empty) ──

/// `focused-pane` reaches `ctx.host.buffers()` (the live host read),
/// proven by NullHost's `PaneHandle::buffer_only(BufferId::default())`
/// return round-tripping through `SteelPane`.
///
/// A stale cached snapshot would still pass this assertion, since both hold
/// the same default on a fresh harness. The live read is guaranteed by the
/// function body calling `ctx.host.buffers()`. See
/// `wire_response_decodes_with_the_requesting_buffers_encoding_not_live_focus`
/// (`hume-editor`) for a case where the two diverge.
#[test]
fn focused_pane_command_mode_returns_steel_pane_id() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = focused_pane(&mut ctx);
    assert!(result.is_ok(), "focused-pane must succeed in command mode");
    assert!(
        matches!(result.unwrap(), SteelVal::Custom(_)),
        "focused-pane must return a SteelVal::Custom (PaneId)"
    );
}

/// `buffers` in command mode returns an empty list (NullHost has no buffers).
#[test]
fn buffers_command_mode_returns_empty_list() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = buffers(&mut ctx);
    assert!(result.is_ok());
    assert!(
        matches!(result.unwrap(), SteelVal::ListV(lst) if lst.is_empty()),
        "buffers must return an empty list when no buffers exist"
    );
}

// ── Selection entry accessors ──────────────────────────────────────────────

fn entry(fields: Vec<SteelVal>) -> SteelVal {
    fields.into_steelval().unwrap()
}

/// A backward selection: anchor after head, covering `2..6`.
fn backward_primary() -> SteelVal {
    entry(vec![
        SteelVal::IntV(5),
        SteelVal::IntV(2),
        SteelVal::IntV(2),
        SteelVal::IntV(6),
        SteelVal::BoolV(true),
    ])
}

#[test]
fn each_selection_accessor_reads_its_own_field() {
    assert_eq!(
        [
            selection_anchor(backward_primary()).unwrap(),
            selection_head(backward_primary()).unwrap(),
            selection_start_char(backward_primary()).unwrap(),
            selection_end_char(backward_primary()).unwrap(),
            selection_primary(backward_primary()).unwrap(),
        ],
        [
            SteelVal::IntV(5),
            SteelVal::IntV(2),
            SteelVal::IntV(2),
            SteelVal::IntV(6),
            SteelVal::BoolV(true),
        ]
    );
}

#[test]
fn an_entry_buffer_selections_builds_reads_back_through_the_accessors() {
    let built = selection_entry(SelectionInfo {
        anchor: 1,
        head: 4,
        start: 1,
        end: 5,
        primary: false,
    });
    assert_eq!(selection_anchor(built.clone()).unwrap(), SteelVal::IntV(1));
    assert_eq!(selection_head(built.clone()).unwrap(), SteelVal::IntV(4));
    assert_eq!(
        selection_start_char(built.clone()).unwrap(),
        SteelVal::IntV(1)
    );
    assert_eq!(
        selection_end_char(built.clone()).unwrap(),
        SteelVal::IntV(5)
    );
    assert_eq!(selection_primary(built).unwrap(), SteelVal::BoolV(false));
}

#[test]
fn a_selection_accessor_raises_for_anything_but_an_entry() {
    let not_entries = [
        SteelVal::BoolV(false),
        entry(vec![SteelVal::IntV(0); 4]),
        entry(vec![
            SteelVal::IntV(0),
            SteelVal::IntV(0),
            SteelVal::IntV(0),
            SteelVal::IntV(1),
            SteelVal::IntV(1),
        ]),
        entry(vec![
            SteelVal::IntV(-1),
            SteelVal::IntV(0),
            SteelVal::IntV(0),
            SteelVal::IntV(1),
            SteelVal::BoolV(true),
        ]),
    ];
    for value in not_entries {
        assert!(
            selection_head(value.clone()).is_err(),
            "{value:?} must be refused"
        );
    }
}
