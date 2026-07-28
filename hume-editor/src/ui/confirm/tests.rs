use super::*;

/// A throwaway `BufferId` — its value is irrelevant to `render_line`, which
/// never reads `action`.
fn dummy_bid() -> BufferId {
    let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
    sm.insert(())
}

#[test]
fn render_line_lists_prompt_then_each_choice_bracketed() {
    let model = ConfirmModel {
        prompt: "foo.rs has changed on disk.".to_string(),
        choices: vec![
            ConfirmChoice {
                key: 'r',
                label: "reload",
            },
            ConfirmChoice {
                key: 'k',
                label: "keep",
            },
        ],
        action: ConfirmAction::ReloadBuffer(dummy_bid()),
    };
    insta::assert_snapshot!(
        model.render_line(),
        @"foo.rs has changed on disk.  [r]reload  [k]keep"
    );
}

#[test]
fn render_line_with_a_single_choice_has_no_trailing_separator() {
    let model = ConfirmModel {
        prompt: "proceed?".to_string(),
        choices: vec![ConfirmChoice {
            key: 'y',
            label: "yes",
        }],
        action: ConfirmAction::ReloadBuffer(dummy_bid()),
    };
    insta::assert_snapshot!(model.render_line(), @"proceed?  [y]yes");
}
