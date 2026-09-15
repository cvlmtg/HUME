use super::*;
use hume_engine::display_lines::line_store::{FormatKey, PaneLineStore};
use hume_engine::pane::WhitespaceConfig;
use hume_engine::providers::ProviderSet;
use hume_rope::line::ContentLine;
use ropey::Rope;

fn map<'a>(
    rope: &'a Rope,
    providers: &'a ProviderSet,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    DisplayLineMap::new(
        rope,
        providers,
        80,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: hume_engine::pane::WrapMode::None,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        store,
    )
}

/// `scroll_cursor_to_display_line` resolves `cursor_char` into a
/// `DisplayLinePos` before handing off to `Viewport::align` — the engine-level
/// behavior `align` itself is exercised by (`hume-engine`'s
/// `display_lines::scroll::tests`); this covers only the char-to-display-line
/// resolution this wrapper adds.
#[test]
fn resolves_cursor_char_before_aligning() {
    let rope = Rope::from_str(&"a\n".repeat(50));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();
    let mut viewport = Viewport::new(80, 24);
    let cursor_char = hume_rope::offset::CharOffset::new(
        hume_rope::lines::line_start_char(&rope, hume_rope::line::RopeyLine::new(25)).index(),
    );

    let mut dlm = map(&rope, &providers, &mut store);
    let geo = viewport.geometry(3).unwrap();
    scroll_cursor_to_display_line(&mut viewport, &mut dlm, geo, cursor_char, 0);

    assert_eq!(
        viewport.top().line,
        ContentLine::new(22),
        "line 25's own position resolves and scrolloff (3) trims target 0 up to margin"
    );
}
