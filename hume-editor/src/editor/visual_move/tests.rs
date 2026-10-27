use super::*;
use crate::editor::Editor;
use crate::editor::commands::FocusedPane;
use hume_engine::pane::{WrapMode, WrapOverride};
use test_fixtures::testing::{parse_state, state};

/// A naive `0..count` loop would hang on `usize::MAX`; the target line runs
/// off the buffer after 4 steps, so this returns at once.
#[test]
fn copy_next_line_count_usize_max_returns_instantly() {
    let (text, sels) = parse_state("-[a]>\nb\nc\nd\ne\n");
    let mut ed = Editor::for_testing_with(state(text, sels));
    let pid = ed.view.focused_pane();
    ed.view.panes[pid].set_wrap(WrapOverride {
        mode: Some(WrapMode::None),
        saved: None,
    });
    let t = FocusedPane::current(&ed.view).pane();
    copy_selection_on_line(&mut ed.state, &mut ed.view, t, usize::MAX, true);
    assert_eq!(ed.current_view().len(), 5); // original + one copy per remaining line
}
