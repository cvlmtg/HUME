# core:markdown — Elements

What each element is drawn from and as, and the node shapes the queries rely on.

## Block elements

| Element | Node | Drawn as |
|---|---|---|
| ATX heading | `atx_hN_marker` | A Braille glyph of N dots and `▌` (`⠈▌`, `⠘▌`, `⠸▌`, `⠹▌`, `⠻▌`, `⠿▌`), scope `markup.heading.N.marker`. The `inline` gets `markup.heading.N.text`, the line `markup.heading.N.line` |
| Setext heading | `setext_heading` | No conceal; the same `.text` scope, and `.line` on every line |
| Bullet | `list_marker_minus`, `_plus`, `_star` | Its first char becomes `•`, unscoped. Ordered markers stay raw |
| Task | `task_list_marker_unchecked`, `_checked` after a bullet, or a box read from the line | Bullet and box become `☐` or `☑`, scope `markup.list.unchecked` or `markup.list.checked`. An ordered item's box stays raw |
| Quote marker | `block_quote_marker`, and each `>` in a `block_continuation` | `┃`, scope `markup.quote` |
| Lazy quote line | a `block_quote` line with no marker | `┃ ` inserted at the first non-blank char |
| Rule | `thematic_break` | `─` × `'rule-width`, scope `punctuation.special` |
| Fence | `fenced_code_block`, its delimiters and `info_string` | Delimiters and info string hidden. Every line gets `markup.raw.block.line`, kept while revealed |

## Inline elements

| Element | Node | Drawn as |
|---|---|---|
| Emphasis, strikethrough, code span | `emphasis_delimiter`, `code_span_delimiter` | Hidden, revealing over its construct's lines |
| Link, image | `inline_link`, `full_reference_link`, `collapsed_reference_link`, `image` | Everything around the `link_text` or `image_description` hidden. No text: raw |
| Shortcut link | `shortcut_link` | Raw: the grammar parses any bracketed text as one, so hiding brackets would strip `[TODO]`, `[!NOTE]`, `[^1]` and `[-]` |
| Autolink | `uri_autolink`, `email_autolink` | `<` and `>` hidden |
| Backslash escape | `backslash_escape` | The backslash hidden |
| Entity | `entity_reference`, `numeric_character_reference` | Its character, unscoped. Raw for a name not in `entities.scm`, or a reference to a control character, a surrogate, or past the last code point |

An image that is a link's whole text keeps both links, since each text comes in a match
with its own link. `render.scm` clamps every conceal to the end of its line's text, since
the setter refuses one covering a line break: a `thematic_break` ends after its line
break, and a link whose destination runs onto a later line hides only to the end of its
first line.

## Tables

`table.scm` draws each `pipe_table` as a grid of `punctuation.special` text. Its conceals,
and every inline conceal inside it, reveal over the whole table.

| Element | Source | Drawn as |
|---|---|---|
| Cell separator | the left cell's trailing blanks and the `\|` | Left padding, ` │ `, right padding |
| Row edge | row start to first cell; last cell's text to row end | `│ ` and padding, or padding and ` │`, inserted when the row has no outer pipe |
| Missing cells | a row short of the table's columns | After the last edge, per missing column, a space, blanks of its width and ` │` |
| Delimiter row | `pipe_table_delimiter_row` | `├─┼─┤`, each `─` run two wider than its column |
| Top, bottom border | the empty line above or below | `┌─┬─┐` or `└─┴─┘` inserted, indented by the display width before the table's first line |

- A column is as wide as its widest cell, measured with `string-width` over the text the
  cell shows: hidden text dropped, replacement text counted. The table has as many columns
  as its longest row, delimiter row included.
- Alignment comes from the delimiter row's `:`s, left by default. Odd centring puts the
  extra blank after.
- One empty line between two tables carries the upper table's bottom border alone.

## Node shapes

Checked against the pinned grammar fixtures (`tests/fixtures/grammars`):

- **Lists:** `list_marker_minus` and its siblings include the space after the marker. A
  marker indented past its parent item's text also holds the blanks past that text's
  column, so `md-preview/marker` starts it at its first non-blank.
  `task_list_marker_unchecked` follows the marker directly, and the space after it belongs
  to the paragraph. A box with nothing after it, as in `- [ ] `, is text, so
  `md-preview/task-box` reads it from the line.
- **Headings:** `atx_hN_marker` is the `#` run alone. The text is an `inline` child,
  absent for an empty heading, holding any closing `#` run. Every `atx_heading` is the
  first child of a `section`; a `setext_heading` is a child of the enclosing section.
- **Quotes:** `block_quote_marker` is `> ` or `>`. Later lines carry a
  `block_continuation` holding every marker of the line (`> > `) and a list item's
  indentation. A lazy line has none.
- **Rules:** `thematic_break` includes its line break.
- **Fences:** `fenced_code_block` spans its fence lines and their line breaks.
  `code_fence_content` runs from after the first content line's `block_continuation` to
  the closing delimiter, so it holds the continuations of later lines in a quote or list
  item. An unclosed fence runs to the end of the document. A `markdown` info string
  injects a Markdown layer.
- **Emphasis:** each `emphasis_delimiter` is one char: `**` is two, and `***a***` is an
  `emphasis` holding a `strong_emphasis`. `~~a~~` is a `strikethrough` holding a
  `strikethrough`, one `~` per side each. A `code_span_delimiter` is the whole backtick
  run.
- **Links:** `link_text` and `image_description` sit between `[` and `]`. A
  `full_reference_link`'s label is a `link_label` with its own brackets. An escape or
  entity can sit inside a `link_destination` or `link_title`.
- **Escapes and entities:** inside a code span, `&` and `\` are anonymous nodes. An
  unknown entity name (`&bogus;`) parses as plain `&` and `;`; every numeric reference is
  a `numeric_character_reference`, whatever its value.
- **Tables:** a `pipe_table` holds a `pipe_table_header`, a `pipe_table_delimiter_row` and
  any `pipe_table_row`s, and runs to the start of the line after its last row. A
  `pipe_table_cell` starts at its text and keeps the blanks after it; `||` holds an empty
  cell. A `pipe_table_delimiter_cell` is the `:`/`-` run alone. An escaped `\|` stays in
  its cell as a `backslash_escape` of the `markdown.inline` layer.
- **Nested tables:** in a list item or quote, each row after the first follows a
  `block_continuation`. A pipe-less line right after a table is one more row; a heading or
  empty line ends it.
