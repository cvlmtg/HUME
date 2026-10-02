use super::*;

/// A throwaway `BufferId`: its value is irrelevant to `render_line`, which
/// never reads `action`.
fn dummy_bid() -> BufferId {
    let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
    sm.insert(())
}

#[test]
fn render_line_lists_prompt_then_each_choice_bracketed() {
    let model = ConfirmLayer {
        prompt: "foo.rs has changed on disk.".to_string(),
        action: ConfirmAction::ReloadBuffer(dummy_bid()),
    };
    insta::assert_snapshot!(
        model.render_line(),
        @"foo.rs has changed on disk.  [r]reload  [k]keep"
    );
}

#[test]
fn a_restore_confirm_targets_only_its_own_buffer() {
    let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
    let (own, other) = (sm.insert(()), sm.insert(()));
    let model = ConfirmLayer {
        prompt: String::new(),
        action: ConfirmAction::RestoreDump(own),
    };

    assert!(model.targets_buffer(own));
    assert!(!model.targets_buffer(other));
}
