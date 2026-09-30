use super::*;
use hume_rope::line::RopeyLine;

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
