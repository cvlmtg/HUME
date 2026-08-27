use super::Editor;
use hume_engine::types::{Modifiers, ResolvedStyle, UnderlineStyle};
use hume_grid::{Rect, Rgb};

/// Render `ed` into a `rect`-sized grid and return a human-readable string.
///
/// Styled runs are wrapped as `<fg=#rrggbb,bg=#rrggbb,bold>text</>`. Unstyled
/// text is emitted bare. Trailing spaces are stripped per row. Used by snapshot
/// tests to lock down the styled render output without a live terminal.
pub(crate) fn render_to_styled_string(ed: &mut Editor, rect: Rect) -> String {
    let buf = ed.render_to_buf(rect);

    let mut out = String::new();
    let default_style = ResolvedStyle::default();

    for y in rect.top()..rect.bottom() {
        let cells: Vec<(ResolvedStyle, &str)> = (rect.left()..rect.right())
            .map(|x| {
                let cell = &buf[(x, y)];
                // A wide glyph's second cell holds no text of its own. It
                // reads as the blank it visually is, so a row's dump is one
                // entry per column either way.
                let text = if cell.is_continuation() {
                    " "
                } else {
                    cell.text()
                };
                (cell.style(), text)
            })
            .collect();

        let last_non_space = cells
            .iter()
            .rposition(|(_, sym)| *sym != " ")
            .map(|i| i + 1)
            .unwrap_or(0);
        let cells = &cells[..last_non_space];

        let mut current = default_style;
        for &(style, sym) in cells {
            if style != current {
                if current != default_style {
                    out.push_str("</>");
                }
                if style != default_style {
                    out.push('<');
                    out.push_str(&style_tag(style));
                    out.push('>');
                }
                current = style;
            }
            out.push_str(sym);
        }
        if current != default_style {
            out.push_str("</>");
        }
        out.push('\n');
    }
    out
}

fn style_tag(style: ResolvedStyle) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = style.fg {
        parts.push(format!("fg={}", color_str(c)));
    }
    if let Some(c) = style.bg {
        parts.push(format!("bg={}", color_str(c)));
    }
    if style.modifiers.contains(Modifiers::BOLD) {
        parts.push("bold".into());
    }
    if style.modifiers.contains(Modifiers::ITALIC) {
        parts.push("italic".into());
    }
    // Every shape reads as one `underline` here. The dump is for spotting
    // regressions in *which cells* are styled, and distinguishing the shapes
    // would churn every existing snapshot without telling a reader anything
    // the theme file doesn't already say.
    if style.underline != UnderlineStyle::None {
        parts.push("underline".into());
    }
    if style.modifiers.contains(Modifiers::REVERSED) {
        parts.push("reverse".into());
    }
    parts.join(",")
}

fn color_str(Rgb(r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}
