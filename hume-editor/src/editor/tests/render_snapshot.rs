use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::Editor;

/// Render `ed` into a `rect`-sized buffer and return a human-readable string.
///
/// Styled runs are wrapped as `<fg=#rrggbb,bg=#rrggbb,bold>text</>`. Unstyled
/// text is emitted bare. Trailing spaces are stripped per row. Used by snapshot
/// tests to lock down the styled render output without a live terminal.
pub(crate) fn render_to_styled_string(ed: &mut Editor, rect: Rect) -> String {
    let buf = ed.render_to_buf(rect);

    let mut out = String::new();
    let default_style = ratatui::style::Style::default();

    for y in rect.top()..rect.bottom() {
        let cells: Vec<(ratatui::style::Style, &str)> = (rect.left()..rect.right())
            .map(|x| {
                let cell = &buf[(x, y)];
                (cell.style(), cell.symbol())
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

fn style_tag(style: ratatui::style::Style) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = style.fg.and_then(color_str) {
        parts.push(format!("fg={s}"));
    }
    if let Some(s) = style.bg.and_then(color_str) {
        parts.push(format!("bg={s}"));
    }
    if style.add_modifier.contains(Modifier::BOLD) {
        parts.push("bold".into());
    }
    if style.add_modifier.contains(Modifier::ITALIC) {
        parts.push("italic".into());
    }
    if style.add_modifier.contains(Modifier::UNDERLINED) {
        parts.push("underline".into());
    }
    if style.add_modifier.contains(Modifier::REVERSED) {
        parts.push("reverse".into());
    }
    parts.join(",")
}

fn color_str(c: Color) -> Option<String> {
    match c {
        Color::Reset => None,
        Color::Black => Some("black".into()),
        Color::Red => Some("red".into()),
        Color::Green => Some("green".into()),
        Color::Yellow => Some("yellow".into()),
        Color::Blue => Some("blue".into()),
        Color::Magenta => Some("magenta".into()),
        Color::Cyan => Some("cyan".into()),
        Color::Gray => Some("gray".into()),
        Color::DarkGray => Some("dark_gray".into()),
        Color::LightRed => Some("light_red".into()),
        Color::LightGreen => Some("light_green".into()),
        Color::LightYellow => Some("light_yellow".into()),
        Color::LightBlue => Some("light_blue".into()),
        Color::LightMagenta => Some("light_magenta".into()),
        Color::LightCyan => Some("light_cyan".into()),
        Color::White => Some("white".into()),
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Indexed(n) => Some(format!("idx{n}")),
    }
}
