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
    let span = sel.range();
    assert!(span.start <= span.end);
    assert_eq!(span.start, co(7));
    assert_eq!(span.end, co(42));
}

#[test]
fn display_line_kind_line_idx() {
    assert_eq!(
        DisplayLineKind::LineStart {
            line_idx: RopeyLine::new(7)
        }
        .line_idx(),
        Some(RopeyLine::new(7))
    );
    assert_eq!(
        DisplayLineKind::Wrap {
            line_idx: RopeyLine::new(7),
            wrap_index: 1
        }
        .line_idx(),
        Some(RopeyLine::new(7))
    );
    assert_eq!(
        DisplayLineKind::Virtual {
            provider_id: 0,
            anchor_line: RopeyLine::new(7)
        }
        .line_idx(),
        None
    );
    assert_eq!(DisplayLineKind::Filler.line_idx(), None);
}

#[test]
fn selection_range_anchor_equals_head() {
    let sel = Selection {
        anchor: co(5),
        head: co(5),
    };
    let span = sel.range();
    assert_eq!(span.start, co(5));
    assert_eq!(span.end, co(5));
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
