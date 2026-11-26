# core:markdown — Architecture

## Decisions

- **Reveal by line:** every line a selection touches shows its source, in every mode, and
  a table or fenced block reveals whole. A revealed heading keeps its bold and band; on
  the primary cursor's line the cursorline background wins over the band.
- **Inserts** (conceals with an empty range) follow the reveal rule: table borders, the
  outer bars of a table without outer pipes, and the bar of a quote line with no `>`.
- **Borders** go on the empty lines around a table, so the table keeps its height when it
  reveals.
- **Vertical motion:** `j` and `k` aim at the target line's revealed layout, since landing
  reveals it. Clicks use the drawn layout. Scrolling moves by drawn rows and lands at the
  column `j` and `k` would use.
- **Header rows:** a pinned heading draws its conceals and its search, diagnostic and
  extra highlights, but no inlay hints, virtual lines, line backgrounds or selections.
- **Rust surface:** `set-conceals!`, `syntax-captures` with `#:ranges`,
  `on-syntax-changed`, and for popups `format-text/<lang>` and `parsed-text-captures`.
  There is no query handle. Link paste rests on `on-paste`, which sees a paste before it
  lands, and renumbering after an edit on `on-text-changed`'s hunks.
- **Inline styles** toggle by the `markdown.inline` tree. When the primary selection is
  inside a span of the style (the innermost one that contains it), one `edit-text!`
  deletes the delimiters of the span around every selection inside one. The grammar
  parses `~~a~~` as a strikethrough inside a strikethrough, and `_*a*_` or `__**a**__` the
  same way, so the span takes along each span of its style whose delimiters touch its own.
  A head on a deleted closing delimiter lands on the character after the text, so when
  every head is on one (a delimiter in its span's second half), `move-left` follows the
  edit; it moves every selection, so it waits for all of them. Otherwise the style is added
  with `surround-add`, once per marker character. Between the two characters of `**` or
  `~~`, `undo-selection-change` gets the wrapped selection back. Selection undo never
  stops on a single cursor, so a lone cursor steps back onto the character it wrapped
  with `move-left` instead.

## Limitations

- Replacement text has no break points and does not wrap: a rule wider than the pane is
  cut, and a table row loses the break points its blanks gave it.
- Under soft wrap, a table wider than the pane draws a broken grid: borders and the
  delimiter row are cut at the pane edge, and rows wrap inside their cells.
- A border needs an empty line (blanks count as text) and a row of its table in the
  render's ranges. A top border also needs the line above its own line in the ranges, or
  no such line: only a returned table tells the plugin where a table ends.
- `string-width` measures a tab at a fixed width, so a tab in a cell breaks the alignment.
- Search, diagnostic and bracket highlights inside hidden text are not drawn on a line no
  selection touches.
- Dragging the mouse across concealed lines reveals them as the selection grows, shifting
  their columns under the pointer.
- The path menu also shows the other completion sources' items for the word before the
  cursor.
- Lines scrolled into view draw raw until the next render, `hook-viewport-debounce-ms`
  after scrolling stops.
- Adding bold or strikethrough is two edits, so undoing it takes two `u`.
- A table or fenced block is fetched and drawn whole by every render that reaches one of
  its rows, so a block longer than the viewport costs its full length each time.

## File layout

| File | Role |
|---|---|
| `manifest.scm` | Lazy activation on the language and the commands |
| `plugin.scm` | Config, hooks, command definitions, keymap and the `markdown:paths` completion source |
| `lib/state.scm` | Per-buffer state (preview on or off, the generation of the last parsed tree), the grammars warned about, `md-preview/box-insert!` and `md-preview/memo` |
| `lib/queries.scm` | The block, inline and list queries, capture accessors, [match grouping](#matches), the task-box rule, conceal order |
| `lib/blocks.scm`, `lib/inline.scm` | Block and inline captures to conceals, line backgrounds and highlights |
| `lib/table.scm` | Table captures to conceals |
| `lib/entities.scm` | The named entities replaced and the numeric references accepted |
| `lib/render.scm` | `md-preview/elements` (the [fenced-block filter](#fenced-blocks), conceal order), and the buffer render |
| `lib/text.scm` | `md-preview/elements` over a popup's text |
| `lib/destinations.scm`, `lib/links.scm`, `lib/paths.scm` | Link destinations, link paste, link-path completion and following links: [`links.md`](links.md) |
| `lib/lists.scm` | List prefixes, ending a list, renumbering, task boxes: [`lists.md`](lists.md) |
| `lib/list-commands.scm` | Enter, `o`, `O`, task toggles and the renumbering pass, applied to a buffer |
| `lib/styles.scm` | The inline-style query, the delimiters a toggle deletes, and the toggle |
| `lib/cells.scm`, `lib/moves.scm` | Reading table rows, and the edits that move rows and columns: [`tables.md`](tables.md) |
| `lib/table-commands.scm` | Selecting cells and columns, and moving rows and columns, applied to a buffer |

`blocks.scm`, `table.scm`, `inline.scm`, `lists.scm`, `cells.scm` and `moves.scm` read no
buffer: lines come in as procedures.

## Rendering

`render-buffer!` is the one path that draws a buffer. It runs on:

- `on-viewport-change`, which Rust debounces by `hook-viewport-debounce-ms`, whenever the
  lines a pane shows change, header rows included;
- `on-syntax-changed`, debounced 50 ms per buffer, after every parse;
- the toggle command turning the preview on.

`on-buffer-open` draws nothing: the first frame and the first parse follow it.
`on-buffer-close` drops the buffer's state, so a pending render does nothing, and when the
buffer stays open with another language, clears the sources and detaches the keymap. A
buffer no pane shows is not drawn.

A render runs the block query over the `markdown` layers, then the inline query over the
`markdown.inline` layers, skipped without that grammar. The inline query also covers every
line of each table found, so a cell crossing the window edge is measured with its inline
conceals.

### Stale trees

The `on-syntax-changed` handler stores `(buffer-generation pane)` before its debounce, and
`render-buffer!` skips the render when the buffer has moved on since: the pending parse
fires the hook again. So a render does not make `syntax-captures` parse on the spot,
except when a parse lands in the same wake as an edit, since the handler then stores the
edited text's generation.

### Ranges

Each render replaces the three `"markdown-preview"` sources (conceals, line backgrounds,
extra highlights) whole, for every pane on the buffer. It covers every range of
`(buffer-viewport-ranges pane)` widened by one line on each side, and every header line as
given. `syntax-captures` returns a match once however many ranges reach it. Lines outside
every range draw raw until a render covers them.

The widening keeps borders stable at the view's edges:

- **Above:** a table ending just above the top line comes back, so the empty top line
  carries its bottom border.
- **Below:** a table starting just below the last line comes back, so the empty line
  before it carries its top border. Under wrapping, a border wider than the pane takes a
  second row and pushes the table's first row off screen, so the border cannot depend on
  that row being shown.
- A header line opens a scope, so it never carries a border and the line above it is not
  fetched.

### Growing hashes

The per-buffer state, a render's line memo, the tables the block elements fill and the
numbers a renumbering hands out grow through `md-preview/box-insert!`, which empties the
box before its `hash-insert`. Steel changes a hash in place only when nothing else holds
it; one still held by its box is copied, and dropping the old version walks every entry,
so each insert would cost the table's size.

## Popups

The editor calls `format-text/markdown` once per popup text. `text.scm` queries the whole
text with `parsed-text-captures` and returns the elements a buffer render would set, with
`'overlaps 'drop`. Nothing is stored and no hook runs. `'rule-width` applies; `'preview`
does not. Nothing in a popup reveals, and the popup wraps the drawn text, so a table wider
than the box breaks its grid.

## Widening by query roots

`syntax-captures` returns a match when its pattern's root overlaps a range, with every
capture of the match, inside the ranges or not. So the root decides how far past the
visible lines an element is fetched:

| Element | Root | Comes back |
|---|---|---|
| Headings | `atx_heading`, `setext_heading` | Those on the given lines, header rows included |
| Fences | `fenced_code_block` | Whole when one line is visible, for the band and the reveal |
| Tables | `pipe_table` | Whole when one row is visible, for the column widths and the reveal |
| Bullets, task boxes | their `list_item` | Those whose item overlaps the ranges |
| Quote markers, continuations, rules | themselves | Those on the visible lines; a code block's continuations also come back with the block ([Fenced blocks](#fenced-blocks)) |
| Inline delimiters | their construct, e.g. `(emphasis (emphasis_delimiter) @span-delimiter) @span` | Both delimiters of an emphasis starting on the last visible line |
| Links, escapes, entities, autolinks | themselves | Those overlapping the ranges |

## Overlapping conceals

An escape or entity inside a link's destination or title lies inside the conceal hiding
that part. `render.scm` sorts conceals by start, longer first, and sets them with
`#:overlaps 'drop`, so the conceal hiding more wins and the dropped one lay in hidden text
anyway.

## Matches

`syntax-captures` returns one match per pattern match, enclosing node first. Where the
plugin needs a child with its parent, the pattern captures the parent too (a fence with
its hidden parts, a table with its rows and cells, a span with its delimiters, a link with
its text, a list with its ordered markers), and tree-sitter repeats the parent in each
child's match. A list marker comes in one match with the task box after it.

- `md-preview/runs` joins adjacent matches sharing a first capture into a run. Matches are
  sorted by their captures, so a parent's matches are adjacent.
- `md-preview/group` files each run under its first capture's name. A table's run groups
  its rows the same way.

## Fenced blocks

The block query also runs over layers injected into code blocks, so a fence whose info
string names `markdown` brings its headings and lists back too. The filter in `render.scm`
drops them:

- It keeps each `code_fence_content` not inside an earlier kept one: the outermost blocks.
- It drops every match whose first capture lies inside a kept content. A match's captures
  lie inside its first, and a block's own matches start outside its content.
- The inline captures go through the same filter, since an injected Markdown layer brings
  its own `markdown.inline` layers.

One match inside a content is kept: a `fence-continuation`, the `block_continuation` that
carries a quote's markers on each line of a code block inside a quote. An injected layer's
continuation lies inside its own block's content too, so a `fence-continuation` is kept
only when its own block's content is the only one containing it.
