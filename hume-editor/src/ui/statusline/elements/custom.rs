use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use crate::editor::Editor;
use crate::ui::theme::EditorColors;

/// The `Custom` element does not implement [`super::StatuslineElement`]:
/// `read`/`format` have no room for the element's own `name`, the same
/// reason [`super::file_path`]'s `render` is a free function instead.
///
/// Renders the focused buffer's last-pushed text for `name` (`(set-
/// statusline-text! name bid text)`), or empty if nothing has been pushed
/// yet — same "absent = empty" convention as every other element.
pub(in crate::ui::statusline) fn render(
    editor: &Editor,
    name: &str,
    colors: &EditorColors,
) -> (Cow<'static, str>, ResolvedStyle) {
    let text = editor
        .state
        .config
        .statusline_text
        .get(&editor.focused_buffer_id())
        .and_then(|by_name| by_name.get(name));
    (
        text.map_or(Cow::Borrowed(""), |s| Cow::Owned(String::from(&**s))),
        colors.statusline,
    )
}
