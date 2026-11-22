# core:markdown — Architecture

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
- **Link paste:** any paste that replaces selected text with a URL or an existing path makes
  a link, whichever key or command pasted it. The kill ring keeps the bare text.
- **Link paths**, pasted or completed, are read from the Markdown file's own folder, the way
  a renderer resolves them, never the working directory.
- **Lists:** off a list or quote line, Enter, `o` and `O` fall back to the editor's own
  behaviour (see [`lists.md`](lists.md)).
- **Rust surface:** `set-conceals!`, `syntax-captures` with `#:ranges`, the
  `on-syntax-changed` hook, and for popups the `format-text/<lang>` command and
  `parsed-text-captures`. There is no query handle. Link paste rests on the `on-paste`
  hook, which sees a paste before it lands. List renumbering after an edit rests on the
  `on-text-changed` hook's hunks.

## Limitations

- Replacement text does not wrap and has no break points: a rule wider than the pane is
  cut, and a table row loses the break points its whitespace gave it.
- Under soft wrap, a table wider than the pane draws a broken grid: borders and the
  delimiter row are cut at the pane edge, and rows wrap inside their cells.
- A border is drawn only on an empty line (blanks count as text) and only while a row of
  its table is in a render's ranges. A top border also needs the line above its own line
  in them, or no such line: only a returned table tells the plugin where a table ends.
- A tab inside a table cell breaks the alignment: `string-width` measures it at a fixed
  tab width.
- Search, diagnostic and bracket highlights inside hidden text are not drawn on a line no
  selection touches.
- Dragging the mouse across concealed lines reveals them as the selection grows, which
  shifts their columns under the pointer.
- The path menu also shows what the other completion sources offer for the word before
  the cursor, such as `core:buffer-words`'s words.
- Lines scrolled into view draw raw until the next render, `hook-viewport-debounce-ms`
  after scrolling stops. The plugin adds no delay of its own.
- A table or fenced block is fetched and drawn whole by every render whose ranges reach
  one of its rows, so a block longer than the viewport costs its full length each time.

## File layout

| File | Role |
|---|---|
| `manifest.scm` | Lazy activation on the language and the commands |
| `plugin.scm` | Config, the hooks, the commands, the `on-paste` handler, the `markdown:paths` completion source and the buffer keymap |
| `lib/state.scm` | Per-buffer state keyed by `(buffer-key pane)` (preview on or off, the generation of the last parsed tree), the grammars warned about, and `md-preview/box-insert!` |
| `lib/queries.scm` | The block, inline and list queries, capture names and accessors, matches grouped by name and into runs (see [Matches](#matches)), the task-box rule, the conceal constructor and order |
| `lib/blocks.scm` | Block captures to conceals, line backgrounds and highlights |
| `lib/table.scm` | Table captures to conceals. Cells are measured with `string-width` |
| `lib/inline.scm` | Inline captures to conceals |
| `lib/entities.scm` | The named entities replaced, and the numeric references accepted |
| `lib/render.scm` | `md-preview/elements` (fenced-block filter, inline ranges with each table's lines, conceal order), and for a buffer the ranges, the line reader, the two `syntax-captures` calls and the three setters |
| `lib/text.scm` | `md-preview/elements` over a popup's text and the result `format-text/markdown` returns |
| `lib/destinations.scm` | A destination written bare or between `<` and `>`, and a path read from the buffer's folder: see [`links.md`](links.md#destinations) |
| `lib/links.scm` | `md-links/paste-link`, the `on-paste` handler's text: see [`links.md`](links.md#link-paste) |
| `lib/paths.scm` | `md-paths/complete!`, the completion source's answer: see [`links.md`](links.md#link-paths) |
| `lib/lists.scm` | A line's prefix, the next line's, ending a list, renumbering, and toggling task boxes: see [`lists.md`](lists.md) |
| `lib/cells.scm` | A row's prefix, pipes and cells read from its text, the table a line is in, and what the cell patterns match on a line: see [`tables.md`](tables.md) |
| `lib/moves.scm` | The edits that move table rows and columns: see [`tables.md`](tables.md#moving-rows-and-columns) |

`blocks.scm`, `table.scm`, `inline.scm`, `lists.scm`, `cells.scm` and `moves.scm` read no buffer: lines
come in as procedures.

## Rendering

`render-buffer!` is the one path that draws a buffer. It is reached from:

- `on-viewport-change`, which Rust debounces by `hook-viewport-debounce-ms` (80 ms by
  default). It fires when the lines a pane shows change, header rows included: a pane's
  first frame on a buffer, a buffer switch, a tab coming back.
- `on-syntax-changed`, after `debounce-by 50` keyed by `buffer-key`, after every parse,
  the first one included.
- The toggle command, when it turns the preview on.

`on-buffer-open` draws nothing: it checks the grammars and attaches the buffer keymap, and the
first frame and the first parse follow it. `on-buffer-close` drops the buffer's state, so a
pending render does nothing, and if the buffer stays open (its language changed away from
`markdown`) clears the three sources and detaches the buffer keymap.

A render runs the block query over the `markdown` layers, then the inline query over the
`markdown.inline` layers. The inline query covers the same ranges plus every line of each
table found, so cells crossing the window edge are measured with their inline conceals.
Without a `markdown.inline` grammar, the inline query is skipped and the block elements
are drawn alone.

### Stale trees

A render runs only while the buffer's tree matches its text:

- The `on-syntax-changed` handler stores `(buffer-generation pane)` at once, before its
  debounce.
- `render-buffer!` compares it with the current generation and skips the render when they
  differ: the pending parse fires `on-syntax-changed` again.
- So a render does not make `syntax-captures` parse on the spot, except when a parse lands
  in the same wake as an edit: the handler then stores the edited text's generation, and a
  render before the next parse parses on the spot.

A buffer no pane shows is not drawn.

### Ranges

Each render replaces the three sources (`"markdown-preview"`: conceals, line backgrounds,
extra highlights) whole, for every pane on the buffer, so it covers every line
`(buffer-viewport-ranges pane)` gives:

- each of its `'ranges`, widened by one line above and one line below;
- each of its `'headers`, the lines of the scope header rows, as given.

Both queries take that list as `#:ranges`; `syntax-captures` returns a match once even
when several ranges reach it. Lines between two panes far apart are not fetched, and lines
outside every range draw raw until a render covers them.

The widening keeps borders stable at the view's edges (see [Tables](elements.md#tables)):

- **Line above:** a table ending just above the pane's top line comes back, so the empty
  top line carries its bottom border.
- **Line below:** a table starting just below the last shown line comes back, so the empty
  line before it carries its top border. Under wrapping, a border wider than the pane
  takes a second row and pushes the table's first row off screen, so the border must not
  depend on that row being shown.
- A header line opens a scope, so it never carries a border, and the line above it is not
  fetched.

### Growing hashes

The per-buffer state, a render's line memo, the tables the block elements fill and the
numbers a renumbering hands out grow through `md-preview/box-insert!`, which empties the box before its `hash-insert`. Steel
changes a hash nothing else holds in place; one still held by its box is copied, and
dropping the old version walks every entry, so each insert would cost the table's size.

## Popups

The editor calls `format-text/markdown` once per popup text and draws what it returns.
`text.scm` queries the tree with `parsed-text-captures` over every line and returns the
elements a buffer render would set, as `'conceals`, `'line-backgrounds` and `'highlights`
with `'overlaps 'drop` (see [Overlapping conceals](#overlapping-conceals)). Nothing is
stored and no hook runs: the call cannot change the editor. `'rule-width` applies;
`'preview` does not.

## Widening by query roots

`syntax-captures` returns a match when its pattern's root node overlaps a range, with every
capture of the match, inside the ranges or not. So the root decides how far past the
visible lines an element is fetched:

| Element | Root | Comes back |
|---|---|---|
| Headings | `atx_heading`, `setext_heading` | Only those on the given lines, a heading in a header row included |
| Fences | `fenced_code_block` | Whole, when one line is visible: both fence lines and the span its band and reveal need |
| Tables | `pipe_table` | Whole, when one row is visible: every row padded to the same widths, every conceal revealing over the table |
| Bullets, task boxes | their `list_item` | Those whose item overlaps the ranges |
| Quote markers, continuations, rules | themselves | Only those on the visible lines. A code block's continuations are also rooted at `code_fence_content` and come back with the block (see [Fenced blocks](#fenced-blocks)) |
| Inline delimiters | their construct, e.g. `(emphasis (emphasis_delimiter) @span-delimiter) @span` | An emphasis starting on the last visible line returns both delimiters |
| Links; escapes, entities, autolinks | the link; themselves | Those overlapping the ranges |

## Overlapping conceals

An escape or entity inside a link's destination or title lies inside the conceal hiding
that part. `render.scm` sorts conceals by start, longer first at one start, and sets them
with `#:overlaps 'drop`, so each one overlapping an earlier one is dropped. The conceal
hiding more wins; the dropped one lay in hidden text anyway.

## Matches

`syntax-captures` returns matches, each the captures of one pattern match, enclosing node
first. Where the plugin needs a child together with its parent, the pattern captures the
parent too: a fence with each of its hidden parts, a table with each row and each cell, a
span with each delimiter, a link with its text, a list with each ordered marker.
Tree-sitter reports one match per child, the parent repeated in each. A list marker comes
in one match with the task box after it, its sibling in their `list_item`.

- `md-preview/runs` joins adjacent matches with one first capture into a run: that
  capture and the rest of each match. Matches are sorted by their captures, so a parent's
  matches are adjacent.
- `md-preview/group` files each run under its first capture's name, the parent's when the
  pattern captures one. A table's run groups its rows the same way, a run per row.

## Fenced blocks

The block query also runs over layers injected into code blocks: a fence whose info
string names `markdown` is parsed as Markdown, and its headings and lists come back too. The filter in
`render.scm` drops them:

- It walks the `code_fence_content` captures in order and keeps each one not inside an
  earlier kept one: the outermost blocks.
- It drops every match whose first capture lies inside a kept content. A match's captures
  come from one layer and lie inside its first, and a block's own matches start outside
  its content and stay.
- The inline captures go through the same filter against the block query's contents: an
  injected Markdown layer brings its own `markdown.inline` layers.

One match inside a content is kept: a `block_continuation` child of
`code_fence_content`, captured as `fence-continuation`. In a code block inside a quote,
these carry the quote's markers on every line after the first. An injected layer's
continuation also lies inside its own block's content, so a `fence-continuation` is kept
only when the outermost content is the one `code_fence_content` containing it.
