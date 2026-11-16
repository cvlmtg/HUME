//! A scope style a script sets, as the script wrote it.

/// One entry of `set-scope-styles!`, decoded but not yet checked: the
/// editor reads its color, modifier and underline names with the theme
/// file's own vocabulary, so a bad name is refused there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeStyleInput {
    pub scope: String,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub underline_color: Option<String>,
    pub underline_style: Option<String>,
    pub modifiers: Vec<String>,
}
