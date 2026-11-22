# Table cells and columns

`markdown/select-cell` (`m i |`) selects the cell under each selection, dropping the
selections outside a table, and changes nothing when no selection is in a cell with text.
`markdown/select-column` (`m |`) selects the cells of the primary selection's column in the
header and body rows, and logs a warning with the count of rows it left out. Both select a
cell's text trimmed of blanks, and a blank cell as a cursor on its last blank.

`markdown/select-around-cell` (`m a |`) selects each cell with the pipe before it and the
blanks after its text, so `d` removes the cell from its row. The first cell of a row
without a leading pipe has no pipe before it and gets its blanks only, and `d` leaves the
row starting with a pipe. When the selections hold every header and body cell with text of
a column, as `m |` leaves them, the column's delimiter cell and its empty cells are targets
too, so `m |`, `m a |`, `d` deletes the column. A table with no body row has its header cell
as the whole column.

The cells come from the line text, not from the `markdown` tree, which reads `||` as an
empty cell in some rows and not in others. For a row such as `| c || d |` the commands see
three cells while the preview, drawn from the tree, shows two.

## The text of a row

A line's prefix is its leading run of blanks, `>` quote markers, and list markers (`-`, `+`,
`*`, or one to nine digits and `.` or `)`) each followed by a blank. A table row can start
with a list marker only when the table is inside that list item, so the prefix is never cell
text. From the end of the prefix, a backslash and the character after it are text, and every
other `|` is a pipe. A line with no pipe is not a row.

A pipe right after the prefix is the leading pipe, and the last pipe is a closing pipe when
only blanks follow it. Every other pipe, and the leading one, opens a cell that runs to the
next pipe or to the end of the line. In a row without a leading pipe, the first cell starts
at the end of the prefix. Two adjacent pipes hold an empty cell.

`md-cells/prefix-end` and `md-cells/pipes` read a line this way, and the two sift patterns
next to them match the same pieces: `m i |`'s matches the prefix and each stretch of text
between pipes, `m a |`'s the prefix, each pipe with the text after it, and a first cell with
no leading pipe. The prefix alternative never matches the empty string: an empty match at the
start of a line would make the next match start one character later.

## Tables

A table is the run of adjacent rows around a line whose delimiter row, a row of cells such as
`---`, `:--` or `--:`, follows the header. The header is the row above the first delimiter row
of the run, and the rows above the header are not part of the table. Lines with pipes and no
delimiter row, such as a shell pipeline in a code block, are not a table.

## The cell under an offset

An offset falls in the first cell whose end, the pipe after it or the end of the line, is at or
after it: a pipe belongs to the cell before it, and the prefix and a leading pipe to the first
cell. Past the last cell, the offset falls in the last cell. An empty cell holds nothing to
select: `markdown/select-cell` drops the selection on it, and `markdown/select-column` counts
its row as skipped.

## Cells and columns

A column is the cell at one index in each header and body row that has that many cells. `m a |`
treats a column as whole when the selections hold all of its cells with text. It then adds the
column's delimiter cell, and the pipe before each of its empty cells, which is what `d` removes
to drop the empty cell from its row.

## Selecting

A plugin moves selections only through editor commands. Each command works out every target
from the text first, so one with no target changes nothing. Then `md-tables/select!` runs:

1. `collapse-selection`, then `select-line` for `m i |`, or `inner-paragraph` for `m |` and
   `m a |`, which need every row of the table. `m |` keeps only the primary selection first.
   A table has no blank line, so its paragraph holds all of it.
2. `split-selection-on-newlines`, one selection per line. `copy-selection-on-next-line`
   would step display lines, and a wide row wraps.
3. `sift-within` with the inner or around pattern. Every target must be one of the
   selections it leaves; the command raises otherwise.
4. Drop the selections that are not targets, make the primary target primary with one
   counted `cycle-primary-forward`, and, for `m i |` and `m |`,
   `trim-selection-whitespace`.

None of these records a jump or recenters the view, which `goto-location!` would. Selection undo steps over the whole
command: the native commands one Steel command runs record one entry.

## Dropping selections

`md-tables/keep!` walks the selections once from the primary, in document order, wrapping. A
run of selections to keep is passed with one `cycle-primary-forward` counted by its length,
and a run to drop is removed with one `remove-primary-selection` counted by its length: each
removal makes the next selection primary. The walk ends on the first kept selection at or
after the one it started from.
