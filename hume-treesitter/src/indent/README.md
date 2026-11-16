# Tree-sitter indentation

The model `indent.rs`, `walk.rs` and `query.rs` implement: the width a line gets from the language's
`indents.scm` and the buffer's syntax tree, for a new line and for the dedent of a closing token.

## Principles

- **Helix's `indents.scm` files are consumed as written.** The engine implements their containment
  semantics from this document.
- **Widths, never whitespace.** The model takes typed positions and returns display widths
  (`BufferLineCol`). `hume-ops` decides when a line break or a typed line consults it, and
  `TabStyle::render_indent` turns a width into tabs and spaces.
- **Typed coordinates.** Bytes, chars and lines convert only through `BufferText`. Lines after the
  planned break are a private `PlannedLine` type, so a pre-insertion line cannot be compared with
  one.
- **Fail soft to copy, fail loud on bugs.** No layer with an indents query, or a split inside a
  multi-line literal, gives `None`, and the caller keeps its own indent. An `indents.scm` that
  does not compile is left out of the grammar, and a pattern using a predicate or property the
  engine does not evaluate is disabled, each reported as a warning. A tree reaching past the
  text's end panics: it was not parsed from that text.

## Engine

All results are widths in display columns, with `tw = max(tab_width, 1)`. One level is
`indent_stop(1, tw)` columns (`hume_rope::width`). A line's width is
`columns().indent(line, tw).1`.

### Target

- **New line** (a break at `split`): `b = char_to_byte(split)`, `L0` is the line of `split`, and the
  target is `PlannedLine(L0) + 1`.
- **Existing line** `L`: a blank line gives `None`. Otherwise `b` is the byte of
  `lines().indent_end(L)` and the target is `PlannedLine(L)`.

A byte's line is `ropey_char_to_line(byte_to_char(x))`. For a new line, `start_line(n)` adds 1 when
`n.start_byte() ≥ b`, and `end_line(n)` adds 1 when `n.end_byte() > b`. Every other comparison uses
pre-insertion lines.

### Layer

1. Walk the layers holding `b..b` innermost first and take the first whose grammar has an indents
   query. If none has one, the result is `None`.
2. While the next layer in that order whose depth is lower (an enclosing layer holding `b`, never a
   sibling at the same depth) has the same language and an indents query, move outward to it. A
   macro's arguments injected as the same language are indented by the enclosing file's tree;
   hybrid baselines compare this merged layer.
3. The root layer's base is `B = 0`.
4. An injected layer's base: find its range `r` holding `b`, and let `La` be the line of `r.start`.
   The base line is `La` when its first non-blank byte is at or after `r.start`, else `La + 1` when
   that line starts inside `r`. `B` is the base line's width, or 0 when there is no base line.

### Query run

1. `node = root.descendant_for_byte_range(b, b)`, or `None`. The root's end byte must not exceed
   the text's length.
2. **Deepest preceding node:** the last child of `node` ending at or before `b`, then repeatedly
   its last child.
3. **Candidate body** (new line only). Start with `x` = the deepest preceding node. While `x` has a
   parent: stop if `x` ends after `b`; if `x.next_sibling()` starts at or after `b`, it is the
   candidate; otherwise move to the parent.
4. **Byte range:** from the deepest preceding node's end minus one (saturating), or `b` when there
   is none, to the candidate's end, or `b + 1` when there is none.
5. One `QueryCursor` over that range with a match limit of 256, the limit the queries are written
   for. Tree-sitter evaluates `#eq?`, `#match?` and `#any-of?` and their negations itself.
6. Matches failing a predicate below are skipped.
7. Captures on MISSING nodes are skipped. Of the rest:
   - the match's anchor is its first `@anchor`;
   - `@indent`, `@indent.always`, `@outdent` and `@outdent.always` set flags on their node, one flag
     per kind however many matches capture it;
   - `@align` records the anchor's column, the display width of the text from the anchor's line
     start to the anchor, only when the match has an anchor;
   - in a header-scoped pattern, an `@indent`, `@indent.always`, `@outdent`, `@outdent.always` or
     anchored `@align` capture sets the node's `header_any`, which drives descent; of those, only
     `@indent` and `@indent.always` also set `header_scope`, which moves the scope start;
   - `@extend` and `@extend.prevent-once` set flags;
   - `@opaque` records its byte range;
   - every other capture is a helper and is ignored.

### Predicates

Checked when the query compiles. The engine evaluates the predicates below. Any other predicate
tree-sitter does not evaluate itself, and any property but `#set! "scope" "header"`, is a compile
error.

| Predicate / property | Rule |
|---|---|
| `#not-kind-eq? @c "k"` | Fails when the first node of `@c` has kind `k`. Holds when `@c` captured no node. |
| `#same-line? @a @b`, `#not-same-line? @a @b` | Both first nodes must exist, otherwise it fails. Compares their planned start lines. |
| `#one-line? @c`, `#not-one-line? @c` | The first node must exist, otherwise it fails. Compares its planned start and end lines. |
| `#set! "scope" "header"` | The only property allowed, and only without a capture. |
| `#is?`, `#is-not?` | Compile error. |

Every predicate of a pattern must hold. A compile error is a `QueryError` of kind `Predicate`
at the pattern's start: its `offset` is `start_byte_for_pattern`, and its `row` and `column` follow
tree-sitter's convention (0-based row, byte column).

### Opaque

When an opaque range holds `b` (`start ≤ b < end`) and starts on a line before `b`'s, the result is
`None` and the caller copies.

### Start node

- When there is a candidate with `header_any`, start from the candidate.
- Otherwise walk from the deepest preceding node toward `node` (exclusive) with `stop = false`.
  For each `x`:
  - prevent-once sets `stop`;
  - `captured` is `x`'s extend flag;
  - `extend` is `captured` and either `x` ends on the target byte's line, or that line's level is
    greater than the level of `x`'s start line;
  - when `captured && stop`, reset `stop`;
  - else when `extend && !stop`, `x` is extended;
  - move to the parent.

  The innermost extended node is the start. The walk continues past it and marks every extended
  node on the way to `node`.
- With neither, start from `node`.

A line's level is the display width of its leading whitespace divided by `tw`, rounded down.

### Containment

Walk from the start node to the root. For each node `n` with flags or an alignment:
- `ns = start_line(n)` and `ne = end_line(n)`;
- the scope start is the parent's planned start when `n` has `header_scope`, else `ns`;
- `n` contains the target when `start < target ≤ ne`. An extended node's scope reaches over the
  new line: it contains the target whenever `start < target`;
- `n` opens the target when `ns == target`.

Then:
- A node with both an indent-type and an outdent-type flag adds no level. Its `@align` still
  applies.
- Otherwise:
  - `@indent` adds 1 when the node contains the target and no earlier node has counted its scope
    start: scopes opening on one line count once;
  - `@indent.always` adds 1 when the node contains the target;
  - the plain `@outdent`s on nodes opening the target subtract 1 together, however many there are;
  - `@outdent.always` subtracts 1 for each node opening the target.
- The innermost containing `@align` wins, and its node's start line is the aligned line. The anchor
  column covers the scopes enclosing the aligned node, so of the `@indent` and `@indent.always`
  counts only those whose scope start is below the aligned line remain. Every outdent remains. A
  list opened on a line below an aligned argument list's first line indents its elements one level
  past the anchor, and its closer returns to the anchor.

The result is `net` levels and the alignment column `A`, if any.

### Widths

- **Absolute:** `W = (A or B) ± indent_stop(|net|, tw)`, saturating at 0. An alignment column is
  measured from column 0, so the base is never added to it.
- **Hybrid** (what both public entry points return):
  - Baselines are lines `L0, L0 − 1, …` for a new line and `L0 − 1, …` for an existing line.
  - Blank lines are skipped without counting. At most 4 non-blank lines are tried.
  - A baseline qualifies when it is in the same layer, gives levels, and has the same alignment.
  - The first qualifying baseline `Lb` gives `W = width(Lb) ± indent_stop(|net_t − net_b|, tw)`,
    saturating. When none qualifies, the absolute width is used.
- **`after_token`** (computed with the width): with `pb` the start of the cluster before the split,
  it is true when the node for `[pb, b)` in the chosen layer is a leaf covering that range and no
  more, whose kind equals the text of `[pb, b)`. It is false at the buffer start.
- **Electric dedent:** for an existing line whose first token may close a scope, take the node at
  `b` with no planned break and run the query over `b..b + 1`. Walk that node and its ancestors
  while they start at the same byte, looking for an outdent-type flag on a complete token. Whether
  a token is complete depends on the trigger landing at the cursor's byte:
  - **Line break:** the token is complete when it ends at or before the cursor's byte. A token
    reaching past it is not: the break carries its rest to the next line. The check reads the
    tree before the break, so a break inside a longer node leaves the line as it is even when the
    text it leaves would complete a token: `else` split from `elsewh` is not dedented.
  - **Typed character:** the token is complete when it ends before the cursor's byte. Otherwise it
    is complete only when:
    - its last character is not a word character as the buffer's `word-chars` classify it, so
      `else` stays open while `elsewhere` could still follow; and
    - its text is not a proper prefix of another anonymous token of the grammar, so `.` stays
      open while `..` could still follow. The prefix set is computed once, when the query
      compiles.

  Without a complete outdent token the result is `None`; otherwise it is the hybrid width of the
  existing line.
