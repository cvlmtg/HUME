# core:markdown

Draws Markdown without its syntax, in the style of Obsidian's live preview. Every line a
selection touches shows its source, and a code block or a table shows its source as a
whole while a selection is inside it.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:markdown" #:config (hash 'preview #t 'rule-width 40))
```

- **Depends on:** `core:stdlib`, for config validation.
- **Activates on:** the first `markdown` buffer, or any of its three commands
  (`manifest.scm`), the first Markdown popup's `format-text/markdown` call included. The
  manifest lists no events: a `#:languages` manifest may list no buffer event.
- **Grammars:** `markdown` and `markdown.inline`. The first buffer opened without one
  logs a warning per missing grammar, once a session, naming `:plum-install-grammar`.

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'preview` | bool | `#t` | Whether the preview starts on for each buffer. Popups are drawn either way |
| `'rule-width` | integer ≥ 1 | `40` | How many `─` a horizontal rule draws. Plugins cannot read the pane width |

## Commands

| Command | Effect |
|---|---|
| `:toggle-markdown-preview` | Turn the preview off for the current buffer, clearing everything it drew, or back on, rendering at once |
| `markdown/toggle-preview` | The same function, bindable |
| `format-text/markdown` | `(format-text/markdown text parsed)`: what the preview draws for a text and its parsed tree, as the hash a text formatter returns. The editor calls it for every Markdown popup and the completion documentation box, and `call!` reaches it |

## Decisions

- **Reveal:** every line a selection touches shows its source, in every mode; a table or
  fenced block reveals whole while a selection is inside it. A revealed heading keeps its
  bold and band, and on the primary cursor's line the cursorline background wins over a
  band.
- **Layout:** `j` and `k` aim at their target line's revealed layout, since landing reveals
  it. Clicks use the drawn layout. Scrolling moves by drawn rows, and the cursor lands on
  its row's text at the column `j` and `k` would use.
- **Inserts:** a conceal with an empty range is an insert, and follows the reveal rule:
  table borders, the outer bars of a table without outer pipes, and the bar of a quote line
  with no `>`.
- **Borders** go on the empty lines next to a table, so the table keeps its height when it
  reveals.
- **Code blocks** get a band over every line. The fences and the info string are hidden;
  the info string picks the highlighting language.
- **Raw:** shortcut links (`[text]`), and Markdown inside a fenced code block.
- **Header rows:** a pinned heading draws like its body line: conceals and the search,
  diagnostic and extra highlights, but no inlay hints, virtual lines, line backgrounds or
  selections.
- **Viewport:** a render covers the visible lines widened to whole syntax nodes (see
  [Widening by query roots](#widening-by-query-roots)), not the whole buffer.
- **Popups** draw every element a buffer does, tables included. Nothing in a popup reveals,
  and the popup wraps the drawn text, so a table wider than the box breaks its grid.
- **Rust surface:** `set-conceals!`, `syntax-captures` with `#:ranges`, the
  `on-syntax-changed` hook, and for popups the `format-text/<lang>` command and
  `parsed-text-captures`. There is no query handle.

## Limitations

- Replacement text does not wrap and has no break points: a rule wider than the pane is
  cut, and a table row loses the break points its whitespace gave it.
- Under soft wrap, a table wider than the pane draws a broken grid: borders and the
  delimiter row are cut at the pane edge, and rows wrap inside their cells.
- A border is drawn only on an empty line (blanks count as text) and only while a row of
  its table is in a render's ranges. A top border also needs the line above its own line
  in them, or no such line.
- A tab inside a table cell breaks the alignment: `string-width` measures it at a fixed
  tab width.
- Search, diagnostic and bracket highlights inside hidden text are not drawn on a line no
  selection touches.
- Dragging the mouse across concealed lines reveals them as the selection grows, which
  shifts their columns under the pointer.
- Lines scrolled into view draw raw until the next render, `hook-viewport-debounce-ms`
  after scrolling stops. The plugin adds no delay of its own.
- A table or fenced block is fetched and drawn whole by every render whose ranges reach
  one of its rows, so a block longer than the viewport costs its full length each time.

## How it works

### Files

| File | Role |
|---|---|
| `manifest.scm` | Lazy activation on the language and the three commands |
| `plugin.scm` | Config, the hooks and the commands |
| `lib/state.scm` | Per-buffer state keyed by `(buffer-key pane)` (preview on or off, the generation of the last parsed tree), the grammars warned about, and `md-preview/box-insert!` |
| `lib/queries.scm` | The block and inline queries, capture names and accessors, grouping by name, the conceal constructor and order |
| `lib/blocks.scm` | Block captures to conceals, line backgrounds and highlights |
| `lib/table.scm` | Table captures to conceals, and each table's line span, which the inline query also covers. Cells are measured with `string-width` |
| `lib/inline.scm` | Inline captures to conceals |
| `lib/entities.scm` | The named entities replaced, and the numeric references accepted |
| `lib/render.scm` | `md-preview/elements` (fenced-block filter, inline ranges, conceal order), and for a buffer the ranges, the two `syntax-captures` calls and the three setters |
| `lib/text.scm` | `md-preview/elements` over a popup's text and the result `format-text/markdown` returns |

`blocks.scm`, `table.scm` and `inline.scm` call no editor function: lines come in as
procedures.

### Rendering

`render-buffer!` is the one path that draws a buffer. It is reached from:

- `on-viewport-change`, which Rust debounces by `hook-viewport-debounce-ms` (80 ms by
  default). It fires when the lines a pane shows change, header rows included: a pane's
  first frame on a buffer, a buffer switch, a tab coming back.
- `on-syntax-changed`, after `debounce-by 50` keyed by `buffer-key`, after every parse,
  the first one included.
- The toggle command, when it turns the preview on.

`on-buffer-open` only checks the grammars: the first frame and the first parse follow it.
`on-buffer-close` drops the buffer's state, so a pending render does nothing, and clears
the three sources if the buffer stays open (its language changed away from `markdown`).

A render runs the block query over the `markdown` layers, then the inline query over the
`markdown.inline` layers. The inline query covers the same ranges plus every line of each
table found, so cells crossing the window edge are measured with their inline conceals.
Without a `markdown.inline` grammar, the inline query is skipped and the block elements
are drawn alone.

#### Stale trees

A render runs only while the buffer's tree matches its text:

- The `on-syntax-changed` handler stores `(buffer-generation pane)` at once, before its
  debounce.
- `render-buffer!` compares it with the current generation and skips the render when they
  differ: the pending parse fires `on-syntax-changed` again.
- So a render does not make `syntax-captures` parse on the spot, except when a parse lands
  in the same wake as an edit: the handler then stores the edited text's generation, and a
  render before the next parse parses on the spot.

A buffer no pane shows is not drawn.

#### Ranges

Each render replaces the three sources (`"markdown-preview"`: conceals, line backgrounds,
extra highlights) whole, for every pane on the buffer, so it covers every line
`(buffer-viewport-ranges pane)` gives:

- each of its `'ranges`, widened by one line above and one line below;
- each of its `'headers`, the lines of the scope header rows, as given.

Both queries take that list as `#:ranges`; `syntax-captures` returns a match once even
when several ranges reach it. Lines between two panes far apart are not fetched, and lines
outside every range draw raw until a render covers them.

The widening keeps borders stable at the view's edges (see [Tables](#tables)):

- **Line above:** a table ending just above the pane's top line comes back, so the empty
  top line carries its bottom border.
- **Line below:** a table starting just below the last shown line comes back, so the empty
  line before it carries its top border. Under wrapping, a border wider than the pane
  takes a second row and pushes the table's first row off screen, so the border must not
  depend on that row being shown.
- A header line opens a scope, so it never carries a border, and the line above it is not
  fetched.

#### Growing hashes

The per-buffer state, a render's line memo and the tables the block elements fill grow
through `md-preview/box-insert!`, which empties the box before its `hash-insert`. Steel
changes a hash nothing else holds in place; one still held by its box is copied, and
dropping the old version walks every entry, so each insert would cost the table's size.

### Popups

The editor calls `format-text/markdown` once per popup text and draws what it returns.
`text.scm` queries the tree with `parsed-text-captures` over every line and returns the
elements a buffer render would set, as `'conceals`, `'line-backgrounds` and `'highlights`
with `'overlaps 'drop` (see [Overlapping conceals](#overlapping-conceals)). Nothing is
stored and no hook runs: the call cannot change the editor. `'rule-width` applies;
`'preview` does not.

### Widening by query roots

`syntax-captures` returns a match when its pattern's root node overlaps a range, with every
capture of the match, inside the ranges or not. So the root decides how far past the
visible lines an element is fetched:

| Element | Root | Comes back |
|---|---|---|
| Headings | `atx_heading`, `setext_heading` | Only those on the given lines, a heading in a header row included |
| Fences | `fenced_code_block` | Whole, when one line is visible: both fence lines and the span its band and reveal need |
| Tables | `pipe_table` | Whole, when one row is visible: every row padded to the same widths, every conceal revealing over the table |
| Bullets, task boxes, quote markers, continuations, rules | themselves | Only those on the visible lines. A code block's continuations are also rooted at `code_fence_content` and come back with the block (see [Fenced blocks](#fenced-blocks)) |
| Inline delimiters | their construct, e.g. `(emphasis (emphasis_delimiter) @span-delimiter)` | An emphasis starting on the last visible line returns both delimiters |
| Links; escapes, entities, autolinks | the link; themselves | Those overlapping the ranges |

### Block elements

Each element is one or two capture names in `queries.scm` and one function in
`blocks.scm`.

| Element | Node | Drawn as |
|---|---|---|
| ATX heading | the `atx_hN_marker` of an `atx_heading` | The marker becomes a Braille glyph of N dots and `▌` (`⠈▌`, `⠘▌`, `⠸▌`, `⠹▌`, `⠻▌`, `⠿▌`), scope `markup.heading.N.marker`; the space after it stays. The `inline` gets `markup.heading.N.text`, the line `markup.heading.N.line` |
| Setext heading | `setext_heading` with `setext_hN_underline` | No conceal: the `inline` gets `markup.heading.N.text`, every line of the heading `markup.heading.N.line` |
| Bullet | `list_marker_minus`, `_plus`, `_star` | Its first char becomes `•`, unscoped, keeping its syntax color. Ordered markers stay raw |
| Task | `task_list_marker_unchecked`, `_checked` | Bullet and box become `☐` or `☑`, scope `markup.list.unchecked` or `markup.list.checked`. A box starts where its bullet ends |
| Quote marker | `block_quote_marker`, and each `>` in a `block_continuation` | `┃`, scope `markup.quote`; the space after it stays |
| Lazy quote line | a `block_quote` line with no marker | `┃ ` inserted at the first non-blank char |
| Rule | `thematic_break` | `─` × `'rule-width`, scope `punctuation.special` |
| Fence | `fenced_code_block`, its `fenced_code_block_delimiter`s and `info_string` | Delimiters and info string hidden, revealing with the block. Every line gets `markup.raw.block.line`, kept while revealed |

A quote's later lines carry a `block_continuation`, not a `block_quote_marker`. It holds
every marker of the line (`> > ` two quotes deep) and a list item's indentation, so a bar
is drawn on each `>` in it.

### Inline elements

| Element | Node | Drawn as |
|---|---|---|
| Emphasis, strikethrough, code span | `emphasis_delimiter` in `emphasis`, `strong_emphasis` or `strikethrough`; `code_span_delimiter` in `code_span` | Each delimiter hidden, revealing over its construct's lines |
| Link, image | `inline_link`, `full_reference_link`, `collapsed_reference_link` with a `link_text`; `image` with an `image_description` | Everything around the text hidden. No text: raw |
| Shortcut link | `shortcut_link` | Raw: the grammar parses any bracketed text as one, so hiding brackets would strip `[TODO]`, `[!NOTE]`, `[^1]` and `[-]` |
| Autolink | `uri_autolink`, `email_autolink` | `<` and `>` hidden |
| Backslash escape | `backslash_escape` | The backslash hidden |
| Entity | `entity_reference`, `numeric_character_reference` | Its character, unscoped. Raw for a name not in `entities.scm`, or a reference to a control character (line break included), a surrogate, or past the last code point |

- `inline.scm` walks the captures in order with a stack of the open spans and links. A
  delimiter belongs to the innermost open span and reveals over its lines. A text belongs
  to the innermost open link starting before it, so an image that is a link's whole text
  is not its own link.
- `render.scm` clamps every conceal to the end of its line's text, since the setter
  refuses one covering a line break. A `thematic_break` ends after its line break, and a
  link whose destination or title runs onto a later line hides only to the end of its
  first line.

### Tables

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
  alone. A top border needs the line above its line in the render's ranges, or no such
  line: only a returned table tells the plugin where a table ends.

### Overlapping conceals

An escape or entity inside a link's destination or title lies inside the conceal hiding
that part. `render.scm` sorts conceals by start, longer first at one start, and sets them
with `#:overlaps 'drop`, so each one overlapping an earlier one is dropped. The conceal
hiding more wins; the dropped one lay in hidden text anyway.

### Fenced blocks

The block query also runs over layers injected into code blocks: a fence whose info
string names `markdown` is parsed as Markdown, and its headings and lists come back too. The filter in
`render.scm` drops them:

- It walks the `code_fence_content` captures in order and keeps each one not inside an
  earlier kept one: the outermost blocks.
- It drops every capture inside a kept content. A block's own captures lie outside its
  content and stay.
- The inline captures go through the same filter against the block query's contents: an
  injected Markdown layer brings its own `markdown.inline` layers.

One capture inside a content is kept: a `block_continuation` child of
`code_fence_content`, captured as `fence-continuation`. In a code block inside a quote,
these carry the quote's markers on every line after the first. An injected layer's
continuation also lies inside its own block's content, so a `fence-continuation` is kept
only when the outermost content is the one `code_fence_content` containing it.

### Node shapes

Checked against the pinned grammar fixtures (`tests/fixtures/grammars`):

- **Lists:** `list_marker_minus` and its siblings include the space after the marker.
  `task_list_marker_unchecked` follows the marker directly; the space after the box
  belongs to the paragraph.
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
