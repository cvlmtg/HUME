use super::*;

fn wp(line: usize, character: usize) -> WirePos {
    WirePos { line, character }
}

#[test]
fn to_lsp_range_round_trips_through_from_lsp_range() {
    let range = ExclusiveRange::new(wp(3, 7), wp(4, 0));
    let lsp_range = to_lsp_range(range).expect("fits u32");
    assert_eq!(
        lsp_range,
        lsp_types::Range {
            start: lsp_types::Position {
                line: 3,
                character: 7
            },
            end: lsp_types::Position {
                line: 4,
                character: 0
            },
        }
    );
    assert_eq!(from_lsp_range(&lsp_range), range);
}

#[test]
fn to_lsp_range_rejects_a_line_past_u32() {
    let range = ExclusiveRange::new(wp(0, 0), wp(u64::from(u32::MAX) as usize + 1, 0));
    assert_eq!(to_lsp_range(range), None);
}

#[test]
fn to_lsp_range_rejects_a_character_past_u32() {
    let range = ExclusiveRange::new(wp(0, 0), wp(0, u64::from(u32::MAX) as usize + 1));
    assert_eq!(to_lsp_range(range), None);
}

#[test]
fn to_json_position_matches_protocol_shape() {
    assert_eq!(
        to_json_position(wp(3, 7)),
        serde_json::json!({"line": 3, "character": 7})
    );
}

#[test]
fn to_json_range_matches_protocol_shape() {
    let range = ExclusiveRange::new(wp(3, 7), wp(4, 0));
    assert_eq!(
        to_json_range(range),
        serde_json::json!({
            "start": {"line": 3, "character": 7},
            "end": {"line": 4, "character": 0},
        })
    );
}
