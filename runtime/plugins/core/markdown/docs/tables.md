# core:markdown — Tables

The table commands read cells from the line text, not from the `markdown` tree, which
reads `||` as an empty cell in some rows and not in others. So for `| c || d |` the
commands see three cells while the preview shows two.

## The text of a row

A line's prefix is its leading run of blanks, `>` markers, and list markers (`-`, `+`,
`*`, or one to nine digits and `.` or `)`) each followed by a blank or the line end. A row
starts with a list marker only when the table is inside that list item, so the prefix is
never cell text. Past the prefix, a backslash and the character after it are text, and
every other `|` is a pipe. A line with no pipe is not a row.

A pipe right after the prefix is the leading pipe, and the last pipe is a closing pipe
when only blanks follow it. Every other pipe, and the leading one, opens a cell that runs
to the next pipe or the line end. Without a leading pipe, the first cell starts at the end
of the prefix. `||` holds an empty cell.

`md-lists/container-of` and `md-cells/pipes` read a line this way, and the two sift
patterns match the same pieces: `m i |`'s the prefix and each run of text between pipes,
`m a |`'s the prefix, each pipe with the text after it, and a first cell with no leading
pipe. A pattern takes a list marker only with a blank after it, which a row always has,
since a marker ending the line leaves no pipe. The prefix alternative never matches the
empty string, which at a line start would push the next match one character on.

## Tables

A table comes from the run of adjacent rows around a line: the run's first delimiter row
(cells such as `---`, `:--`, `--:`) below its top line, the header right above it, and the
rest of the run. Rows above the header are not part of it, and a run with no delimiter
row, such as a shell pipeline in a code block, is not a table.

## The cell under an offset

An offset falls in the first cell whose end, its closing pipe or the line end, is at or
after it: a pipe belongs to the cell before it, the prefix and a leading pipe to the first
cell, and anything past the last cell to the last cell. An empty cell has nothing to
select, so `m i |` drops a selection on it and `m |` counts its row as skipped.

## Whole columns

A column is the cell at one index in each header and body row that has that many cells.
When the selections hold all of a column's cells with text, `m a |` also takes its
delimiter cell and the pipe before each of its empty cells, so `d` drops the whole column.

## Selecting

A plugin moves selections only through editor commands. Each command first works out every
target from the text, so one with no target changes nothing, then `md-tables/select!`:

1. `collapse-selection`, then `select-line` for `m i |`, or `inner-paragraph` for `m |`
   and `m a |`, which need every row. A table holds no blank line, so its paragraph is the
   whole table. `m |` first keeps only the primary selection, and `m a |` one selection
   per table, since `inner-paragraph` reads the whole table from each.
2. `split-selection-on-newlines`, one selection per line. `copy-selection-on-next-line`
   would step display lines, and a wide row wraps.
3. `sift-within` with the inner or around pattern. Every target must be one of the
   selections it leaves, or the command raises.
4. `md-tables/keep!` drops the selections that are not targets, then one counted
   `cycle-primary-forward` makes the primary target primary. `m i |` and `m |` end with
   `trim-selection-whitespace`.

`md-tables/keep!` walks the selections once from the primary, in order and wrapping,
passing a run to keep with one counted `cycle-primary-forward` and dropping a run with one
counted `remove-primary-selection`, since each removal makes the next selection primary.

None of these commands records a jump or recenters the view, as `goto-location!` would,
and selection undo steps over the whole command.

## Moving rows and columns

A block is a run of adjacent rows, or columns, of one table that the selections touch.
`edit-text!` lands a selection on replaced or deleted text where the new text starts, and
a plugin can rebuild selections only through editor commands, so a move never edits the
text under the selections. It moves the row or column next to each block across the block,
in one `edit-text!`: every selection follows its text, an `m a |` selection keeps its
pipe, and one `u` undoes the move.

- **Rows `a..b` down:** row `b+1`'s text and a line break are inserted at the start of
  `a`, and the text from `b`'s line break to `b+1`'s is deleted, which never takes the
  buffer's final line break.
- **Up:** row `a-1` and its line break are deleted, and inserted after `b`'s line break,
  past a line selection ending on it.
- **Cells `i..j` right:** cell `j+1` with the pipe before it is deleted and inserted
  before cell `i`'s pipe. A first cell with no pipe gets the cell inserted before its
  text, with a pipe after it.
- **Left:** cell `i-1` is deleted with the pipe before it, the pipe after it staying as
  cell `i`'s, and inserted after cell `j`'s text with a pipe before it. A first cell with
  no pipe is deleted with the pipe after it.

A cell's text is everything between its pipes, so its blanks move with it, and in a row
without outer pipes the first and last cells carry their blanks to the other end.

A selection touches the table lines from its start's to its last character's. A row move
takes those lines. A column move takes, on each line, the cells whose text overlaps the
selection there, or else the cell under its start, as `m i |` reads it. The table's
columns are its delimiter row's cells.

Nothing moves when no selection touches a table, a row move touches the header or
delimiter row, or a block would pass its table's edge (the first body row or the last row,
the first or last column). A column move also needs, in every row, the cells it swaps;
when a row has fewer, nothing moves and a warning gives the count.
