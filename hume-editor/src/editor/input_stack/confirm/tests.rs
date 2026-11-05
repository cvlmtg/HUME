use super::*;

#[test]
fn render_line_lists_prompt_then_each_choice_bracketed() {
    let [bid] = hume_core::fresh_ids();
    let model = ConfirmLayer::for_test(
        "foo.rs has changed on disk.".to_string(),
        ConfirmAction::ReloadBuffer(bid),
    );
    insta::assert_snapshot!(
        model.render_line(),
        @"foo.rs has changed on disk.  [r]reload  [k]keep"
    );
}

#[test]
fn a_restore_confirm_targets_only_its_own_buffer() {
    let [own, other] = hume_core::fresh_ids();
    let model = ConfirmLayer::for_test(String::new(), ConfirmAction::RestoreDump(own));

    assert!(model.targets_buffer(own));
    assert!(!model.targets_buffer(other));
}
