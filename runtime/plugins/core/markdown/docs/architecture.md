# core:markdown — Architecture

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
- **Link paste:** any paste that replaces selected text with a URL makes a link, whichever
  key or command pasted it. The kill ring keeps the bare URL.
- **Link paths:** completed relative to the Markdown file's own folder, the way a renderer
  resolves them, never the working directory.
- **Rust surface:** the editor functions `set-conceals!` and `syntax-captures` (with its
  `#:ranges`), the `on-syntax-changed` hook, and for popups the `format-text/<lang>`
  command the editor calls and `parsed-text-captures`. There is no query handle. Link
  paste rests on the `on-paste` hook, which sees a paste before it lands.

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
- A link path is inserted as the file is named: a name with a space makes a destination
  Markdown reads as text, not as a link.
- The path menu also shows what the other completion sources offer for the word before
  the cursor, such as `core:buffer-words`'s words.
- Lines scrolled into view draw raw until the render that covers them runs, which is
  `hook-viewport-debounce-ms` after the scrolling stops. The plugin adds no delay setting
  of its own.
- A table or fenced block larger than the viewport is fetched and drawn whole by every
  render that shows one of its rows, so such a render costs as much as the block is long.

## File layout

| File | Role |
|---|---|
| `manifest.scm` | Lazy activation on the language and the three commands |
| `plugin.scm` | Config, the hooks, the commands, the `on-paste` handler and the `markdown:paths` completion source |
| `lib/state.scm` | Per-buffer state, keyed by `(buffer-key pane)`: whether the preview is on, and the generation the buffer's last parsed tree was built from. The grammar names warned about this session, and `md-preview/box-insert!` |
| `lib/queries.scm` | The block and inline queries, their capture names, the capture accessors, the grouping of captures by name, and the conceal constructor and order |
| `lib/blocks.scm` | Block captures to conceals, line backgrounds and highlights. Calls no editor builtin: lines come in as procedures |
| `lib/table.scm` | Table captures to conceals, and the line spans of the tables, which the inline query also covers. Reads no buffer either: lines come in as procedures, and cells are measured with `string-width` |
| `lib/inline.scm` | Inline captures to conceals. Calls no editor builtin either |
| `lib/entities.scm` | The named entities the plugin replaces, and the numeric references it accepts |
| `lib/render.scm` | The render funnel: `md-preview/elements` (the fenced-block filter, the inline captures' ranges, the conceal order) over the captures and line procedures it is handed, and for a buffer the pane ranges, the two `syntax-captures` calls and the three setters |
| `lib/text.scm` | `md-preview/elements` over a popup's text: its lines, the two `parsed-text-captures` calls, and the result `format-text/markdown` returns |
| `lib/links.scm` | `md-links/paste-link`, the `on-paste` handler's text: see [`links.md`](links.md#link-paste) |
| `lib/paths.scm` | `md-paths/items`, the completion source's answer: see [`links.md`](links.md#link-paths) |

## Rendering

`render-buffer!` is the one path that draws a buffer. Three callers reach it:

- `on-viewport-change`, with the pane in hand. Rust debounces it by
  `hook-viewport-debounce-ms` (80 ms by default), and it fires when the lines a pane shows
  change, its scope header rows included, as on a pane's first frame on a buffer, a buffer
  switch, and a tab coming back.
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
the lines `(buffer-viewport-ranges pane)` gives at once: each of its `'ranges`, starting
one line above the first line it names, and each of its `'headers`, the lines of the
panes' scope header rows, as given. Both queries take the list as `#:ranges`, and
`syntax-captures` returns a match once even when several ranges reach it, such as a table
or a code block two panes both show part of. The lines between two panes far apart are not
fetched, and lines outside every range are drawn raw until a render covers them. The line
above a range is in it so that a table ending just above a pane's first shown line comes
back: the empty line after it, on the pane's first row below its scope headers, carries
that table's bottom border (see [Tables](elements.md#tables)). A header line opens a scope, so it is
never the empty line a border is drawn on, and the line above it is not fetched.

The per-buffer state, a render's line memo and the tables the block elements fill grow
through `md-preview/box-insert!`, which empties the box before its `hash-insert`. Steel
changes a hash that nothing else holds in place. A hash still held by its box is copied
instead, and dropping the version the box held then walks every entry, so each insert would
cost the size of the table.

## Popups

The editor calls `format-text/markdown` once per popup text, with the text and its parsed
tree, and draws what it returns. `text.scm` reads the lines from the string, queries the
tree with `parsed-text-captures` over every line, and returns the elements a buffer render
would set as its result's `'conceals`, `'line-backgrounds` and `'highlights`, with
`'overlaps 'drop` (see [Overlapping conceals](#overlapping-conceals)). Nothing is stored
and no hook runs: the call reads the editor and cannot change it. `'rule-width` applies,
and `'preview` does not.

## Widening by query roots

`syntax-captures` returns a match when the node its pattern is rooted at overlaps one of
the ranges, with every capture of the match, inside the ranges or not. So each pattern's root
decides how far past the visible lines an element is fetched:

- ATX headings are rooted at `atx_heading` and setext headings at `setext_heading`, so
  only the ones on the given lines come back. A heading drawn in a scope header row is one
  of them, since its line is in the ranges.
- Fences are rooted at `fenced_code_block`. A block one line of which is visible comes back
  whole: both fence lines, and the line span its band and reveal need.
- Tables are rooted at `pipe_table`, their rows and cells included. A table one row of
  which is visible comes back whole, so every row is padded to the same widths and every
  conceal reveals over the whole table.
- Bullets, task boxes, quote markers, block continuations and rules are rooted at
  themselves, so only the ones on the visible lines come back. The continuations a code
  block's content holds are also captured rooted at `code_fence_content`, so they come
  back with the whole block (see [Fenced blocks](#fenced-blocks)).
- Inline delimiters are rooted at their construct: `(emphasis (emphasis_delimiter) @span-delimiter)`.
  An emphasis that starts on the last visible line and ends below it returns both
  delimiters. Links are rooted at the link, escapes, entities and autolinks at themselves.

## Overlapping conceals

Inline conceals can overlap: an escape or an entity inside a link's destination or title
lies inside the conceal hiding that part. `render.scm` sorts the conceals by start, the
longer first at one start, and sets them with `#:overlaps 'drop`, so the setter keeps
each one that overlaps no conceal before it in that order. The conceal hiding more wins,
and the one it drops lay inside text that is hidden anyway.

## Fenced blocks

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
