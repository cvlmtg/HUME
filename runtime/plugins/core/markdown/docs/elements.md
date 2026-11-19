# core:markdown — Elements

What each element of the preview is drawn from and drawn as, and the node shapes the queries rely on.

## Block elements

Captures are grouped by name. Each element below is one or two capture names in
`queries.scm` and one function in `blocks.scm`.

| Element | Node | Drawn as |
|---|---|---|
| ATX heading | the `atx_hN_marker` of an `atx_heading` | The marker becomes the level as a Braille glyph of one to six dots followed by `▌` (`⠈▌`, `⠘▌`, `⠸▌`, `⠹▌`, `⠻▌`, `⠿▌`), scope `markup.heading.N.marker`. The space after it stays. The heading's `inline` gets `markup.heading.N.text` and its line `markup.heading.N.line` |
| Setext heading | `setext_heading` with `setext_hN_underline` | No conceal: the text's `inline` gets `markup.heading.N.text`, and every line of the heading, underline included, `markup.heading.N.line` |
| Bullet | `list_marker_minus`, `_plus`, `_star` | Its first char becomes `•`, unscoped, so the bullet keeps its syntax color. Ordered markers stay raw |
| Task | `task_list_marker_unchecked`, `_checked` | The bullet and the box become `☐` or `☑`, scope `markup.list.unchecked` or `markup.list.checked`. A box starts where its bullet ends |
| Quote marker | `block_quote_marker`, and each `>` in a `block_continuation` | The `>` becomes `┃`, scope `markup.quote`. The space after it stays |
| Lazy quote line | a line of a `block_quote` with no marker on it | `┃ ` inserted at the line's first non-blank char |
| Rule | `thematic_break` | `─` repeated `'rule-width` times, scope `punctuation.special` |
| Fence | `fenced_code_block`, its `fenced_code_block_delimiter`s and its `info_string` | Both delimiters and the info string are hidden and reveal with the block's lines. Every line of the block gets `markup.raw.block.line`, which stays while the block is revealed |

A quote marker's continuation on a later line of the same paragraph is a
`block_continuation`, not a `block_quote_marker`. A continuation holds every marker of its
line, `> > ` for a line two quotes deep, and the indentation of a list item when there is
one, so the plugin draws a bar on each `>` in it.

## Inline elements

| Element | Node | Drawn as |
|---|---|---|
| Emphasis, strikethrough, code span | `emphasis_delimiter` in `emphasis`, `strong_emphasis` or `strikethrough`; `code_span_delimiter` in `code_span` | Each delimiter is hidden and reveals over the lines of the construct it belongs to |
| Link, image | `inline_link`, `full_reference_link`, `collapsed_reference_link` with a `link_text`; `image` with an `image_description` | Everything before the text and everything after it is hidden. A link or image with no text stays raw |
| Shortcut link | `shortcut_link` | Raw. The inline grammar parses any bracketed text as one, definition or not, so hiding its brackets would also strip `[TODO]`, `[!NOTE]`, `[^1]` and `[-]` |
| Autolink | `uri_autolink`, `email_autolink` | `<` and `>` are hidden |
| Backslash escape | `backslash_escape` | The backslash is hidden |
| Entity | `entity_reference`, `numeric_character_reference` | Replaced by its character, unscoped. A name outside the table in `entities.scm` stays raw, and so does a numeric reference to a control character (a line break included), a surrogate, or a value past the last code point |

`inline.scm` walks the captures in order with a stack of the spans and links that are
still open at each capture. A delimiter belongs to the innermost open span, whose lines
it reveals over. A text belongs to the innermost open link starting before it: an image
that is a link's whole text has the same range as that text, and is not its link.

Every conceal ends at most at the end of its line's text: `render.scm` clamps each one
before the setter, which refuses a conceal covering a line break. A `thematic_break` node
ends after its line break. A link whose destination or title runs onto a later line hides
only up to the end of its first line.

## Tables

`table.scm` draws each `pipe_table` as a box grid of `punctuation.special` text. Every
conceal it sets reveals over the table's lines, and so does every inline conceal inside the
table, whatever its own reveal span: a selection on any row shows the whole table raw.

| Element | Node | Drawn as |
|---|---|---|
| Cell separator | the run between two `pipe_table_cell`s' text: the trailing blanks of the left cell and the `\|` | The left cell's padding, ` │ `, and the right cell's padding |
| Row edge | the run from a row's start to its first cell, and from its last cell's text to its end | `│ ` and the first cell's padding, or the last cell's padding and ` │`. A row without an outer pipe gets the same text inserted at the row's start or end |
| Missing cells | a row with fewer cells than the table has columns | After the row's last edge, each missing column as blanks of its width between a space and ` │` |
| Delimiter row | `pipe_table_delimiter_row` | `├─┼─┤`, each column's `─` run two wider than the column |
| Top border | the line above the table, when it exists and is empty | `┌─┬─┐` inserted at its start, indented by the display width of what precedes the table on its first line |
| Bottom border | the line below the table, when it exists and is empty | `└─┴─┘`, the same way |

A cell's text runs from the cell's start to its last non-blank char: the grammar leaves
the blanks before a cell's text out of the cell and the blanks after it in. A column is as
wide as its widest cell, measured with `string-width` over the text the cell shows: the
cell's own inline conceals applied, hidden text dropped and replacement text counted. The
delimiter row's cells give each column its alignment: a `:` at the start aligns it left,
at the end right, at both ends centre, and none left. When a centred cell's padding is
odd, the extra blank goes after it. The table has as many columns as its longest row, delimiter row included.

One empty line between two tables carries the upper table's bottom border alone. A top
border is set only when the line above its line lies in one of the render's ranges or does
not exist: the plugin knows that a table ends on a line only when the block query returned
that table.

The table-specific limitations are under [Limitations](architecture.md#limitations).

## Node shapes

Checked by parsing samples with the pinned grammar fixtures (`tests/fixtures/grammars`):

- `list_marker_minus` and its siblings include the space after the marker. The task box
  `task_list_marker_unchecked` follows the marker directly; the space after the box belongs
  to the paragraph.
- `atx_hN_marker` is the `#` run alone. The heading's text is an `inline` child, absent for
  an empty heading. A closing `#` run is part of the `inline`.
- Every `atx_heading` is the first child of a `section`, a heading inside a quote
  included. `setext_heading` is a child of the enclosing section.
- `block_quote_marker` is `> ` or `>`. On a quote's later lines the markers are a
  `block_continuation`, which may hold several (`> > `). A lazy line has none.
- `thematic_break` includes the line break after it.
- `fenced_code_block` spans its fence lines and their line breaks. `info_string` follows
  the opening delimiter. `code_fence_content` runs from the first content line, after that
  line's `block_continuation`, to the closing delimiter, so it holds the continuations of
  every later line of a block inside a quote or a list item. An unclosed fence runs to the
  end of the document.
- A fence whose info string is `markdown` injects a markdown layer over its content.
- Each `emphasis_delimiter` is one char: `**` is two delimiters, and `***a***` is an
  `emphasis` holding a `strong_emphasis`. `~~a~~` is a `strikethrough` holding a
  `strikethrough`, one `~` each side of each. A `code_span_delimiter` is the whole backtick
  run.
- A link's `link_text` and an image's `image_description` sit between `[` and `]`. A link
  without text has neither. A `full_reference_link`'s label is a `link_label` holding its
  own brackets; a `collapsed_reference_link` ends in a bare `[]`.
- An escape or an entity can sit inside a `link_destination` or a `link_title`. Inside a
  code span, `&` and `\` are anonymous nodes.
- The inline grammar parses an entity name it does not know (`&bogus;`) as plain `&` and
  `;` tokens, and every numeric reference as a `numeric_character_reference`, whatever its
  value.
- A `pipe_table` holds a `pipe_table_header`, a `pipe_table_delimiter_row` and any
  `pipe_table_row`s, and runs to the start of the line after its last row. The pipes are
  anonymous `|` nodes. A `pipe_table_cell` starts at its text and keeps the blanks after
  it; a cell of blanks only keeps them all, and `||` holds an empty cell. A
  `pipe_table_delimiter_cell` is the `:`/`-` run alone. A row without outer pipes starts
  and ends at its cells. An escaped `\|` stays inside its cell, as a `backslash_escape` of
  the cell's `markdown.inline` layer.
- A table inside a list item or a quote starts after its first line's indentation or
  marker; each later row follows a `block_continuation`. A line without a pipe right after
  a table is one more row; a heading or an empty line ends the table.
