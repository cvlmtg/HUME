use super::*;
use crate::editor::input_stack::PickerLayer;
use crate::editor::input_stack::command::CommandLayer;
use crate::editor::input_stack::confirm::{ConfirmAction, ConfirmLayer};
use crate::editor::input_stack::drawer::{DrawerLayer, DrawerSelect};
use crate::editor::input_stack::menu::MenuLayer;
use crate::editor::replay::WaitingInput;
use steel::rvals::SteelVal;

fn menu(label: &str) -> MenuLayer {
    MenuLayer::for_test(
        crate::editor::tests::non_empty_strings(&[label]),
        hume_scripting::Callback::for_test(SteelVal::BoolV(false)),
    )
}

fn confirm() -> ConfirmLayer {
    ConfirmLayer::for_test(
        "test?".to_string(),
        ConfirmAction::ReloadBuffer(hume_core::BufferId::default()),
    )
}

fn popup_model(text: &str) -> PopupLayer {
    PopupLayer::new(
        text.to_string(),
        None,
        hume_core::PopupLayout::Cursor,
        &mut hume_engine::theme::Theme::default(),
        &hume_engine::theme::ScopeRegistry::new(),
    )
}

#[test]
fn truncate_refuses_base_and_returns_empty() {
    let mut stack = InputStack::new();
    let base = stack.top();
    let removed = stack.truncate(base);
    assert!(removed.is_empty());
    assert!(stack.is::<BaseLayer>(stack.top()));
}

#[test]
fn truncate_to_base_keeps_index_zero() {
    let mut stack = InputStack::new();
    stack.push(menu("m"));
    stack.push(confirm());
    stack.truncate_to_base();
    assert_eq!(stack.top().depth, 0);
    assert!(stack.is::<BaseLayer>(stack.top()));
}

#[test]
fn truncate_returns_top_first() {
    let mut stack = InputStack::new();
    stack.push(menu("bottom"));
    let r = stack.top();
    stack.push(confirm());
    let removed = stack.truncate(r);
    assert_eq!(removed.len(), 2);
    assert!(removed[0].is::<ConfirmLayer>());
    assert!(removed[1].is::<MenuLayer>());
}

#[test]
fn layer_ref_is_stale_after_truncate_then_push_at_same_depth() {
    let mut stack = InputStack::new();
    let first = stack.push(menu("first"));
    stack.truncate(first);
    let second = stack.push(menu("second"));

    assert_eq!(first.depth, second.depth, "same index, different id");
    assert!(!stack.is_live(first), "stale ref must not read as live");
    assert!(stack.is_live(second));
}

#[test]
fn ref_of_finds_topmost_of_kind() {
    let mut stack = InputStack::new();
    assert_eq!(stack.ref_of::<MenuLayer>(), None);
    let r = stack.push(menu("only"));
    assert_eq!(stack.ref_of::<MenuLayer>(), Some(r));
}

#[test]
fn is_settled_for_true_on_a_fresh_stack() {
    // `PickerLayer` is never pushed in this group: an arbitrary witness
    // type, standing in for `is_settled_for`'s base case (nothing modal
    // open at all), same contract the deleted `is_stack_settled` tested
    // directly.
    let stack = InputStack::new();
    assert!(stack.is_settled_for::<PickerLayer>());
}

#[test]
fn is_settled_for_false_with_an_unrelated_modal_overlay_above_the_mode_layer() {
    let mut stack = InputStack::new();
    stack.push(menu("m"));
    assert!(!stack.is_settled_for::<PickerLayer>());
}

#[test]
fn is_settled_for_true_again_once_the_overlay_is_truncated() {
    let mut stack = InputStack::new();
    let r = stack.push(menu("m"));
    stack.truncate(r);
    assert!(stack.is_settled_for::<PickerLayer>());
}

#[test]
fn is_settled_for_true_with_a_mode_layer_alone_on_top() {
    let mut stack = InputStack::new();
    stack.push(CommandLayer::new(WaitingInput::for_test()));
    assert!(stack.is_settled_for::<PickerLayer>());
}

#[test]
fn is_settled_for_tolerates_its_own_type_buried_under_a_non_modal_overlay() {
    // The case `is_settled_for` exists for, beyond `is_stack_settled`'s
    // old all-or-nothing contract: a prior `Menu` instance still counts
    // as settled for `Menu` even with a later non-modal `Drawer` on top
    // of it.
    let mut stack = InputStack::new();
    stack.push(menu("m"));
    stack.push(DrawerLayer::new(
        crate::editor::tests::drawer_items(&["d"]),
        DrawerSelect::Steel(hume_scripting::Callback::for_test(SteelVal::BoolV(false))),
        0,
    ));
    assert!(stack.is_settled_for::<MenuLayer>());
}

#[test]
fn fall_through_after_self_truncate_reaches_the_layer_below() {
    // `fall_through` itself lives on `Editor` (`input_stack/walk.rs`) and
    // needs a whole editor to exercise end to end. This test pins the
    // `InputStack` half of its contract: `below(r)` stays addressable
    // by depth alone even after `r` itself was just removed, since
    // every index below `r.depth` never moved.
    let mut stack = InputStack::new();
    let base = stack.top();
    let r = stack.push(menu("m"));
    stack.truncate(r);
    assert_eq!(stack.below(r), base);
}

#[test]
fn popup_prefers_a_popup_layer_over_a_lower_sticky_slot() {
    let mut stack = InputStack::new();
    *stack.sticky_popup_slot_mut().expect("Base has a slot") = Some(popup_model("sticky on base"));
    stack.push(popup_model("scrollable layer"));
    assert_eq!(
        stack.popup().map(|p| p.text.as_str()),
        Some("scrollable layer")
    );
}

#[test]
fn popup_reads_only_the_current_mode_layers_slot_not_a_buried_one() {
    // Base's own slot holding a value must not leak through once a
    // different mode layer is on top. `push_mode_layer` clears it
    // before taking over for exactly this reason; this pins the read
    // side independent of that write-time behavior.
    let mut stack = InputStack::new();
    *stack.sticky_popup_slot_mut().expect("Base has a slot") = Some(popup_model("buried on base"));
    stack.push(CommandLayer::new(WaitingInput::for_test()));
    assert!(stack.popup().is_none());
}

#[test]
fn sticky_popup_slot_mut_is_none_under_a_minibuf_mode_layer() {
    let mut stack = InputStack::new();
    stack.push(CommandLayer::new(WaitingInput::for_test()));
    assert!(stack.sticky_popup_slot_mut().is_none());
}

#[test]
fn clear_popups_clears_both_a_popup_layer_and_the_current_slot() {
    let mut stack = InputStack::new();
    *stack.sticky_popup_slot_mut().expect("Base has a slot") = Some(popup_model("sticky"));
    stack.push(popup_model("scrollable"));
    stack.clear_popups();
    assert!(stack.popup().is_none());
    assert!(stack.is::<BaseLayer>(stack.top()));
}

#[test]
fn clear_popup_layer_leaves_the_sticky_slot_intact() {
    // A completion layer's narrower need than `clear_popups`: evict
    // a pushed `Popup` layer without dismissing a `Sticky` popup sitting
    // in the current mode layer's own slot.
    let mut stack = InputStack::new();
    *stack.sticky_popup_slot_mut().expect("Base has a slot") = Some(popup_model("sticky"));
    stack.push(popup_model("scrollable"));
    stack.clear_popup_layer();
    assert!(stack.is::<BaseLayer>(stack.top()), "the layer home is gone");
    assert_eq!(
        stack.popup().map(|p| p.text.as_str()),
        Some("sticky"),
        "the sticky slot must survive"
    );
}

#[test]
fn truncate_to_base_clears_the_sticky_slot() {
    let mut stack = InputStack::new();
    *stack.sticky_popup_slot_mut().expect("Base has a slot") = Some(popup_model("sticky"));
    stack.push(menu("m"));
    stack.truncate_to_base();
    assert!(stack.popup().is_none());
}
