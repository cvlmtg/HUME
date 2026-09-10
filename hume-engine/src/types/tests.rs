use super::*;
use hume_rope::line::RopeyLine;
use hume_rope::offset::CharOffset;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

#[test]
fn selection_range_ordered() {
    let sel = Selection {
        anchor: co(42),
        head: co(7),
    };
    let (start, end) = sel.range();
    assert!(start <= end);
    assert_eq!(start, co(7));
    assert_eq!(end, co(42));
}

#[test]
fn row_kind_line_idx() {
    assert_eq!(
        RowKind::LineStart {
            line_idx: RopeyLine::new(7)
        }
        .line_idx(),
        Some(RopeyLine::new(7))
    );
    assert_eq!(
        RowKind::Wrap {
            line_idx: RopeyLine::new(7),
            wrap_row: 1
        }
        .line_idx(),
        Some(RopeyLine::new(7))
    );
    assert_eq!(
        RowKind::Virtual {
            provider_id: 0,
            anchor_line: RopeyLine::new(7)
        }
        .line_idx(),
        None
    );
    assert_eq!(RowKind::Filler.line_idx(), None);
}

#[test]
fn selection_range_anchor_equals_head() {
    let sel = Selection {
        anchor: co(5),
        head: co(5),
    };
    let (start, end) = sel.range();
    assert_eq!(start, co(5));
    assert_eq!(end, co(5));
}

#[test]
fn selection_is_collapsed() {
    assert!(
        Selection {
            anchor: co(0),
            head: co(0)
        }
        .is_collapsed()
    );
    assert!(
        !Selection {
            anchor: co(0),
            head: co(1)
        }
        .is_collapsed()
    );
}
