# core:markdown-preview

Draws Markdown without its syntax, in the style of Obsidian's live preview. Every line a selection touches shows its source, and a code block or a table shows its source as a whole while a selection is inside it.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:markdown-preview" #:config (hash 'enabled #t 'rule-width 40))
```

- **Depends on:** `core:stdlib`, for config validation.
- **Activates on:** the first buffer whose language is `markdown`, or any of its three
  commands (`manifest.scm`), the first Markdown popup's `format-text/markdown` call
  included. The manifest lists no events: a `#:languages` manifest may list no buffer
  event, and the language is the trigger.
- **Grammars:** `markdown` and `markdown.inline`. The first buffer opened without one of
  them logs one warning per missing grammar per session, naming the
  `:plum-install-grammar` command that installs it.

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'enabled` | bool | `#t` | Whether the preview starts on for each buffer. Popups are drawn either way |
| `'rule-width` | integer ≥ 1 | `40` | How many `─` a horizontal rule draws. Plugins cannot read the pane width |

## Commands

| Command | Effect |
|---|---|
| `:toggle-markdown-preview` | Turn the preview off for the current buffer, clearing everything it drew, or back on, rendering at once |
| `markdown-preview/toggle` | The same function, bindable |
| `format-text/markdown` | `(format-text/markdown text parsed)`: what the preview draws for a Markdown text, given the text and its parsed tree, as the hash a text formatter returns. The editor calls it for every Markdown popup and the completion documentation box, and `call!` reaches it |

## Decisions

The choices the plugin and the Rust surface under it are built on:

- **Reveal:** every line a selection touches shows its source, in every mode. A table or a
  fenced block reveals as a whole while a selection is inside it. A revealed line keeps its
  heading bold and band: only the hidden or replaced text comes back, and on the primary
  cursor's line the cursorline background wins over a band.
- **Layout:** `j` and `k` resolve their target line against its revealed layout, because
  landing on a line reveals it. Mouse clicks use the drawn layout. Scrolling moves by drawn
  rows, and the cursor lands on the text of its row at the column `j` and `k` would use.
- **Inserts:** a conceal with an empty range is an insert that follows the reveal rule.
  Table borders, the outer bars of a table written without outer pipes, and the bar of a
  quote line with no `>` are inserts.
- **Borders:** a table's top and bottom borders are drawn on the empty lines next to it, so
  the table keeps its height when it reveals.
- **Code blocks:** a background band over every line of the block. The fence characters
  and the info string are hidden: the info string picks the block's highlighting language.
- **Shortcut links** (`[text]`) stay raw, and so does Markdown inside a fenced code block.
- **Header rows:** a pinned heading draws its level marker and bold text like the same line in the
  body. Header rows draw conceals and the search, diagnostic and extra highlights, and no
  inlay hints, virtual lines, line backgrounds or selections.
- **Viewport:** a render covers the visible lines widened to whole syntax nodes (see
  [Widening by query roots](#widening-by-query-roots)), not the whole buffer.
- **Popups:** a Markdown popup and the completion documentation box draw every element a
  buffer does, tables included. A popup has no selection, so nothing in it reveals, and the
  popup wraps the drawn text, so a table wider than the box breaks its grid.
- **Rust surface:** the editor functions `set-conceals!` and `syntax-captures` (with its
  `#:ranges`), the `on-syntax-changed` hook, and for popups the `format-text/<lang>`
  command the editor calls and `parsed-text-captures`. There is no query handle.

## Limitations

- Replacement text does not wrap and has no break points. A rule wider than the pane is
  cut, and a table row loses the break points its whitespace gave it.
- A table wider than the pane draws a broken grid under soft wrap: borders and the delimiter
  row are cut at the pane edge, and a row wraps inside its cells.
- A border is drawn only while a row of its table is in one of a render's ranges, and only
  on an empty line: a line with any text, blanks included, gets none. A top border whose
  line lies just above a range is not drawn either.
- A tab inside a table cell breaks the column alignment: `string-width` measures it at a
  fixed tab width.
- Search, diagnostic and bracket highlights inside hidden text are not drawn on a line no
  selection touches.
- Dragging the mouse across concealed lines reveals them as the selection grows, which
  shifts their columns under the pointer.
- Lines scrolled into view draw raw until the render that covers them runs, which is
  `hook-viewport-debounce-ms` after the scrolling stops. The plugin adds no delay setting
  of its own.
- A table or fenced block larger than the viewport is fetched and drawn whole by every
  render that shows one of its rows, so such a render costs as much as the block is long.

## How it works

### Files

| File | Role |
|---|---|
| `manifest.scm` | Lazy activation on the language and the three commands |
| `plugin.scm` | Config, the hooks and the commands |
| `lib/state.scm` | Per-buffer state, keyed by `(buffer-key pane)`: whether the preview is on, and the generation the buffer's last parsed tree was built from. The grammar names warned about this session, and `md-preview/box-insert!` |
| `lib/queries.scm` | The block and inline queries, their capture names, the capture accessors, the grouping of captures by name, and the conceal constructor and order |
| `lib/blocks.scm` | Block captures to conceals, line backgrounds and highlights. Calls no editor builtin: lines come in as procedures |
| `lib/table.scm` | Table captures to conceals, and the line spans of the tables, which the inline query also covers. Reads no buffer either: lines come in as procedures, and cells are measured with `string-width` |
| `lib/inline.scm` | Inline captures to conceals. Calls no editor builtin either |
| `lib/entities.scm` | The named entities the plugin replaces, and the numeric references it accepts |
| `lib/render.scm` | The render funnel: `md-preview/elements` (the fenced-block filter, the inline captures' ranges, the conceal order) over the captures and line procedures it is handed, and for a buffer the pane ranges, the two `syntax-captures` calls and the three setters |
| `lib/text.scm` | `md-preview/elements` over a popup's text: its lines, the two `parsed-text-captures` calls, and the result `format-text/markdown` returns |

### Rendering

`render-buffer!` is the one path that draws a buffer. Three callers reach it:

- `on-viewport-change`, with the pane in hand. Rust debounces it by
  `hook-viewport-debounce-ms` (80 ms by default), and it fires when a pane's visible lines
  change, including a pane's first frame on a buffer, a buffer switch, and a tab coming
  back.
- `on-syntax-changed`, after `debounce-by 50` keyed by `buffer-key`. It fires after every
  parse of the buffer, the first one included.
- The toggle command, when it turns the preview on.

`on-buffer-open` renders nothing: the pane's first frame and the first parse both follow
it. It only checks the grammars. `on-buffer-close` drops the buffer's state, so a render
still waiting on its debounce does nothing, and clears the three sources when the buffer
stays open, as it does when its language changes away from `markdown`.

A render queries the `markdown` layers with the block query, then the `markdown.inline`
layers with the inline query. The inline query covers the same ranges, plus every line of
each table the block query returned, so the cells of a table crossing the window edge are
measured with their inline conceals too. Without a `markdown.inline` grammar the
render skips the inline query, which would raise, and draws the block elements alone.

A render runs only while the buffer's tree matches its text. The `on-syntax-changed`
handler stores `(buffer-generation pane)` at once, before its debounce, and
`render-buffer!` compares it with the buffer's current generation. When they differ, the
text has changed since the last parse, and the render is skipped: the parse that is
pending fires `on-syntax-changed` again. A render therefore does not make `syntax-captures`
parse the buffer on the spot, with one exception: when a parse lands in the same wake as a
key that edits the buffer, the handler runs after the edit and stores the edited text's
generation, so a render before the next parse lands parses on the spot. A buffer that no
pane shows is not drawn.

Each render replaces the plugin's three sources (`"markdown-preview"` for conceals, line
backgrounds and extra highlights) as a whole, for every pane on the buffer. So it covers
the lines `(buffer-viewport-ranges pane)` names at once, each range starting one line
above the first line it names. Both queries take the list as `#:ranges`, and
`syntax-captures` returns a match once even when several ranges reach it, such as a
section or a code block two panes both show part of. The lines between two panes far
apart are not fetched, and lines outside every range are drawn raw until a render covers
them. The line above a range is in it so that a table ending just above a pane's first
shown line comes back: the empty line after it, at the top of the pane, carries that
table's bottom border (see [Tables](#tables)).

The per-buffer state, a render's line memo and the tables the block elements fill grow
through `md-preview/box-insert!`, which empties the box before its `hash-insert`. Steel
changes a hash that nothing else holds in place. A hash still held by its box is copied
instead, and dropping the version the box held then walks every entry, so each insert would
cost the size of the table.

### Popups

The editor calls `format-text/markdown` once per popup text, with the text and its parsed
tree, and draws what it returns. `text.scm` reads the lines from the string, queries the
tree with `parsed-text-captures` over every line, and returns the elements a buffer render
would set as its result's `'conceals`, `'line-backgrounds` and `'highlights`, with
`'overlaps 'drop` (see [Overlapping conceals](#overlapping-conceals)). Nothing is stored
and no hook runs: the call reads the editor and cannot change it. `'rule-width` applies,
and `'enabled` does not.

### Widening by query roots

`syntax-captures` returns a match when the node its pattern is rooted at overlaps one of
the ranges, with every capture of the match, inside the ranges or not. So each pattern's root
decides how far past the visible lines an element is fetched:

- Headings are rooted at `section`: `(section (atx_heading (atx_h1_marker) @heading.1))`.
  Every section that overlaps a range returns its heading, ancestors included, so the
  heading of a section scrolled into a sticky header row keeps its level marker and bold text.
- Fences are rooted at `fenced_code_block`. A block one line of which is visible comes back
  whole: both fence lines, and the line span its band and reveal need.
- Tables are rooted at `pipe_table`, their rows and cells included. A table one row of
  which is visible comes back whole, so every row is padded to the same widths and every
  conceal reveals over the whole table.
- Bullets, task boxes, quote markers, block continuations and rules are rooted at
  themselves, so only the ones on the visible lines come back. The continuations a code
  block's content holds are also captured rooted at `code_fence_content`, so they come
  back with the whole block (see [Fenced blocks](#fenced-blocks)).
- Setext headings are rooted at `setext_heading`. The markdown grammar opens no section for
  them.
- Inline delimiters are rooted at their construct: `(emphasis (emphasis_delimiter) @span-delimiter)`.
  An emphasis that starts on the last visible line and ends below it returns both
  delimiters. Links are rooted at the link, escapes, entities and autolinks at themselves.

### Block elements

Captures are grouped by name. Each element below is one or two capture names in
`queries.scm` and one function in `blocks.scm`.

| Element | Node | Drawn as |
|---|---|---|
| ATX heading | the `atx_hN_marker` of a section's `atx_heading` | The marker becomes the level as a Braille glyph of one to six dots followed by `▌` (`⠈▌`, `⠘▌`, `⠸▌`, `⠹▌`, `⠻▌`, `⠿▌`), scope `markup.heading.N.marker`. The space after it stays. The heading's `inline` gets `markup.heading.N.text` and its line `markup.heading.N.line` |
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

### Inline elements

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

### Tables

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

The table-specific limitations are under [Limitations](#limitations).

### Overlapping conceals

Inline conceals can overlap: an escape or an entity inside a link's destination or title
lies inside the conceal hiding that part. `render.scm` sorts the conceals by start, the
longer first at one start, and sets them with `#:overlaps 'drop`, so the setter keeps
each one that overlaps no conceal before it in that order. The conceal hiding more wins,
and the one it drops lay inside text that is hidden anyway.

### Fenced blocks

The block query also runs over the layers injected into code blocks: a fence whose info
string names `markdown` is parsed as markdown, and its headings and lists come back with
the rest. The filter in `render.scm` drops them. It walks the `code_fence_content`
captures in order, keeps each one that does not lie inside an earlier kept one (the
outermost blocks, nested fences inside them included), and drops every capture lying
inside a kept content. The block's own captures lie outside its content and are kept.
The inline captures go through the same filter, against the contents the block query
found: an injected markdown layer brings its own `markdown.inline` layers.

One kind of capture inside a content is kept: a `block_continuation` that is a child of
`code_fence_content`, captured as `fence-continuation`. In a code block inside a quote, the
content holds the quote's markers of every line after the first content line, and these
continuations carry them. A capture of the root layer lies inside the outermost content
alone. An injected layer's continuation inside its own code block lies inside that
block's content too, so a `fence-continuation` is kept only when the outermost content is
the one `code_fence_content` capture containing it.

### Node shapes

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
