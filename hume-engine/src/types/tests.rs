use super::*;

#[test]
fn selection_range_ordered() {
    let sel = Selection {
        anchor: 42,
        head: 7,
    };
    let (start, end) = sel.range();
    assert!(start <= end);
    assert_eq!(start, 7);
    assert_eq!(end, 42);
}

#[test]
fn row_kind_line_idx() {
    assert_eq!(RowKind::LineStart { line_idx: 7 }.line_idx(), Some(7));
    assert_eq!(
        RowKind::Wrap {
            line_idx: 7,
            wrap_row: 1
        }
        .line_idx(),
        Some(7)
    );
    assert_eq!(
        RowKind::Virtual {
            provider_id: 0,
            anchor_line: 7
        }
        .line_idx(),
        None
    );
    assert_eq!(RowKind::Filler.line_idx(), None);
}

#[test]
fn selection_range_anchor_equals_head() {
    let sel = Selection { anchor: 5, head: 5 };
    let (start, end) = sel.range();
    assert_eq!(start, 5);
    assert_eq!(end, 5);
}

#[test]
fn selection_is_collapsed() {
    assert!(Selection { anchor: 0, head: 0 }.is_collapsed());
    assert!(!Selection { anchor: 0, head: 1 }.is_collapsed());
}

#[test]
fn editor_mode_cursor_is_bar() {
    assert!(!EditorMode::Normal.cursor_is_bar());
    assert!(!EditorMode::Extend.cursor_is_bar());
    assert!(EditorMode::Insert.cursor_is_bar());
    assert!(EditorMode::Command.cursor_is_bar());
    assert!(EditorMode::Search.cursor_is_bar());
    assert!(EditorMode::Select.cursor_is_bar());
}
