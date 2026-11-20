# core:markdown — Elements

What each element of the preview is drawn from and drawn as, and the node shapes the queries rely on.

## Block elements

| Element | Node | Drawn as |
|---|---|---|
| ATX heading | the `atx_hN_marker` of an `atx_heading` | The marker becomes a Braille glyph of N dots and `▌` (`⠈▌`, `⠘▌`, `⠸▌`, `⠹▌`, `⠻▌`, `⠿▌`), scope `markup.heading.N.marker`; the space after it stays. The `inline` gets `markup.heading.N.text`, the line `markup.heading.N.line` |
| Setext heading | `setext_heading` with `setext_hN_underline` | No conceal: the `inline` gets `markup.heading.N.text`, every line of the heading `markup.heading.N.line` |
| Bullet | `list_marker_minus`, `_plus`, `_star` | Its first char becomes `•`, unscoped, keeping its syntax color. Ordered markers stay raw |
| Task | `task_list_marker_unchecked`, `_checked` after a bullet in its `list_item`, or a box with nothing after it (`- [ ] `), read from the line (see [Node shapes](#node-shapes)) | Bullet and box become `☐` or `☑`, scope `markup.list.unchecked` or `markup.list.checked`. An ordered item's box stays raw |
| Quote marker | `block_quote_marker`, and each `>` in a `block_continuation` | `┃`, scope `markup.quote`; the space after it stays |
| Lazy quote line | a `block_quote` line with no marker | `┃ ` inserted at the first non-blank char |
| Rule | `thematic_break` | `─` × `'rule-width`, scope `punctuation.special` |
| Fence | `fenced_code_block`, its `fenced_code_block_delimiter`s and `info_string` | Delimiters and info string hidden, revealing with the block. Every line gets `markup.raw.block.line`, kept while revealed |

A quote's later lines carry a `block_continuation`, not a `block_quote_marker`. It holds
every marker of the line (`> > ` two quotes deep) and a list item's indentation, so a bar
is drawn on each `>` in it.

## Inline elements

| Element | Node | Drawn as |
|---|---|---|
| Emphasis, strikethrough, code span | `emphasis_delimiter` in `emphasis`, `strong_emphasis` or `strikethrough`; `code_span_delimiter` in `code_span` | Each delimiter hidden, revealing over its construct's lines |
| Link, image | `inline_link`, `full_reference_link`, `collapsed_reference_link` with a `link_text`; `image` with an `image_description` | Everything around the text hidden. No text: raw |
| Shortcut link | `shortcut_link` | Raw: the grammar parses any bracketed text as one, so hiding brackets would strip `[TODO]`, `[!NOTE]`, `[^1]` and `[-]` |
| Autolink | `uri_autolink`, `email_autolink` | `<` and `>` hidden |
| Backslash escape | `backslash_escape` | The backslash hidden |
| Entity | `entity_reference`, `numeric_character_reference` | Its character, unscoped. Raw for a name not in `entities.scm`, or a reference to a control character (line break included), a surrogate, or past the last code point |

- Each delimiter comes in a match with its span and reveals over that span's lines; each
  text comes with its link, so an image that is a link's whole text keeps both links (see
  [Matches](architecture.md#matches)).
- `render.scm` clamps every conceal to the end of its line's text, since the setter
  refuses one covering a line break. A `thematic_break` ends after its line break, and a
  link whose destination or title runs onto a later line hides only to the end of its
  first line.

## Tables

`table.scm` draws each `pipe_table` as a box grid of `punctuation.special` text. Its
conceals, and every inline conceal inside the table, reveal over the table's lines: a
selection on any row shows the whole table raw.

| Element | Node | Drawn as |
|---|---|---|
| Cell separator | the left cell's trailing blanks and the `\|` | Left padding, ` │ `, right padding |
| Row edge | row start to first cell; last cell's text to row end | `│ ` and padding, or padding and ` │`. Without an outer pipe, the same text is inserted |
| Missing cells | a row short of the table's columns | After the last edge, each missing column as blanks of its width between a space and ` │` |
| Delimiter row | `pipe_table_delimiter_row` | `├─┼─┤`, each `─` run two wider than its column |
| Top border | the line above, when it exists and is empty | `┌─┬─┐` inserted, indented by the display width before the table on its first line |
| Bottom border | the line below, when it exists and is empty | `└─┴─┘`, the same way |

- **Cell text** runs from the cell's start to its last non-blank char: the grammar leaves
  the blanks before the text out of the cell and the blanks after it in.
- **Width:** a column is as wide as its widest cell, measured with `string-width` over the
  text the cell shows: hidden text dropped, replacement text counted. The table has as many columns as its
  longest row, delimiter row included.
- **Alignment** comes from the delimiter row: `:` at the start is left, at the end right,
  at both ends centre, none left. Odd centring padding puts the extra blank after.
- **Borders:** one empty line between two tables carries the upper table's bottom border
  alone.

The table-specific limitations are under [Limitations](architecture.md#limitations).

## Node shapes

Checked against the pinned grammar fixtures (`tests/fixtures/grammars`):

- **Lists:** `list_marker_minus` and its siblings include the space after the marker.
  Ordered items carry a `list_marker_dot` or `list_marker_parenthesis`, which the preview
  leaves raw and [`lists.md`](lists.md#renumbering) renumbers.
  `task_list_marker_unchecked` follows the marker directly; the space after the box
  belongs to the paragraph. A box with nothing after it, as in `- [ ] `, is text, so
  `md-preview/task-box` finds it in the line instead.
- **Headings:** `atx_hN_marker` is the `#` run alone. The text is an `inline` child,
  absent for an empty heading, and holds any closing `#` run. Every `atx_heading` is the
  first child of a `section`, inside a quote too; a `setext_heading` is a child of the
  enclosing section.
- **Quotes:** `block_quote_marker` is `> ` or `>`. Later lines carry a
  `block_continuation`, which may hold several markers (`> > `). A lazy line has none.
- **Rules:** `thematic_break` includes its line break.
- **Fences:** `fenced_code_block` spans its fence lines and their line breaks.
  `info_string` follows the opening delimiter. `code_fence_content` runs from the first
  content line, after its `block_continuation`, to the closing delimiter, so it holds the
  continuations of later lines in a quote or list item. An unclosed fence runs to the end
  of the document. A `markdown` info string injects a Markdown layer over the content.
- **Emphasis:** each `emphasis_delimiter` is one char: `**` is two, and `***a***` is an
  `emphasis` holding a `strong_emphasis`. `~~a~~` is a `strikethrough` holding a
  `strikethrough`, one `~` per side each. A `code_span_delimiter` is the whole backtick
  run.
- **Links:** `link_text` and `image_description` sit between `[` and `]`; a link without
  text has neither. A `full_reference_link`'s label is a `link_label` with its own
  brackets; a `collapsed_reference_link` ends in a bare `[]`. An escape or entity can sit
  inside a `link_destination` or `link_title`.
- **Escapes and entities:** inside a code span, `&` and `\` are anonymous nodes. An unknown
  entity name (`&bogus;`) parses as plain `&` and `;` tokens; every numeric reference is a
  `numeric_character_reference`, whatever its value.
- **Tables:** a `pipe_table` holds a `pipe_table_header`, a `pipe_table_delimiter_row` and
  any `pipe_table_row`s, and runs to the start of the line after its last row. Pipes are
  anonymous `|` nodes. A `pipe_table_cell` starts at its text and keeps the blanks after
  it; a blank cell keeps them all, and `||` holds an empty cell. A
  `pipe_table_delimiter_cell` is the `:`/`-` run alone. A row without outer pipes starts
  and ends at its cells. An escaped `\|` stays in its cell, as a `backslash_escape` of the
  cell's `markdown.inline` layer.
- **Nested tables:** in a list item or quote, a table starts after its first line's
  indentation or marker, and each later row follows a `block_continuation`. A pipe-less
  line right after a table is one more row; a heading or empty line ends it.
