# HUME demo scenes, sourced by tools/demo/record.sh, which defines type_text,
# press, pause, screen_shows, wait_screen and show. See tools/demo/README.md.
#
# The recording edits real files of this repo (hume-grid/src/geometry.rs and
# user-manual/docs/getting-started.md), and record.sh quits with :qa! once the
# scenes end, so nothing is saved.
#
# Story: jump across the workspace with the LSP symbol pickers, add a method
# with auto-pairs, auto-indent and a completion with its documentation box,
# wrap words in backticks with surround, toggle a comment, rename with sift,
# hoist two lines with a few multi-cursor edits, and step back through the
# session in the undo tree. Then open the Steel plugin through the file picker,
# the manual page in a split below it, and run the plugin's command on the
# manual's table.

# ── Launch (off-camera) ───────────────────────────────────────────────────────
# rust-analyzer answers a workspace symbol query with the crates it has loaded
# so far, and the picker does not ask again, so each attempt reopens it until
# the answer lists the whole workspace: rect_matches is the count of the whole
# workspace's matches for Rect. The counter reads "0/0 …" while the answer is
# pending, and the count ends at the picker's border once it is in. Each
# attempt types the query only once the picker is open, since with no server
# `zS` opens none and the query's keys would edit the buffer.
rect_matches='symbols: Rect +37/37'
warm_deadline=$((SECONDS + 300))
until
  type_text "zS"
  screen_shows 'workspace symbols:' 2 &&
    type_text "Rect" &&
    screen_shows 'symbols: Rect +[0-9]+/[0-9]+[^ 0-9]|timed out' 30 &&
    screen_shows "$rect_matches" 0
do
  if ((SECONDS >= warm_deadline)); then
    echo "scenes.sh: the workspace symbol picker did not show /$rect_matches/ within 300 s" >&2
    exit 1
  fi
  press Escape
  pause 2000
done
press Escape
pause 1200
show

# ── Scene 1 — workspace and document symbol pickers, inlay hints, hover ──────
# The workspace picker lists matches alphabetically, so Rect's definition in
# geometry.rs is the sixteenth row, below the picker's fifteen visible rows;
# fifteen Downs reach it.
pause 1200
type_text "zS"
pause 400
type_text "Rect"
wait_screen "$rect_matches" 20
pause 600
press Down 15
pause 300
press Enter
pause 800
type_text "zs"
pause 400
type_text "cent"
pause 1200
press Enter
pause 1500
type_text "/min"
press Enter
pause 300
type_text "K"
pause 300
press Ctrl+D
pause 300
press Ctrl+D
pause 300
press Ctrl+D
pause 300
press Escape
pause 400

# ── Scene 2 — a new method: auto-pairs, auto-indent and a completion ─────────
# `.` is a trigger character for rust-analyzer, so the menu opens without a
# key; `ating_su` narrows it to saturating_sub, whose docs show in the box
# beside the menu. Accepting inserts the bare name, without parens. The first
# Escape closes the signature popup, the second leaves Insert mode.
type_text ":102"
press Enter
type_text "o"
press Enter
type_text "/// Removes rows from the bottom of self, saturating at a height of zero." 45
press Enter
type_text "pub fn without_bottom(self, rows: u16) -> Rect {" 45
press Enter
type_text "Rect {" 45
press Enter
type_text "height: self.height.satur" 60
pause 600
type_text "ating_su"
pause 600
press Ctrl+D
pause 1200
press Enter
type_text "(rows),"
press Enter
type_text "..self"
pause 900
press Escape
pause 300
press Escape
pause 500

# ── Scene 3 — surround: backticks around three words at once ─────────────────
# `s` sifts the doc line down to one cursor per match; `m w` adds the pair the
# next key names.
type_text ":104"
press Enter
type_text "x"
type_text "s"
type_text '\b(rows|self|height)\b'
pause 500
press Enter
pause 700
type_text "mw"
type_text '`'
pause 1200

# `G G` toggles a comment on the selected function; a second press undoes it.
type_text ":106"
press Enter
type_text "maf"
pause 600
type_text "GG"
pause 600
type_text "GG"
pause 300
type_text ";"

# ── Scene 4 — document symbols, then rename with sift ────────────────────────
# `m i p` selects the doc comment and the function together, so the doc's
# mention of the parameter is renamed with the six others. Ctrl+O returns to
# the previous jump.
type_text "zs"
pause 400
type_text "contains"
pause 1000
press Enter
pause 800
type_text "mip"
pause 400
type_text "s"
type_text '\bpos\b'
pause 500
press Enter
pause 800
type_text "c"
type_text "cell"
press Escape
pause 800
type_text "u"
pause 400
press Ctrl+O 2
pause 800

# ── Scene 5 — move lines and turn them into bindings ─────────────────────────
# `x` then Ctrl+X selects two lines. They paste above `Rect {` one level too
# deep, and `<` dedents the pasted selection. `S` splits it into one cursor per
# line, `I` inserts at each line's start, and `x` re-selects each cursor's line
# for the next sift. The struct literal then takes shorthand fields.
type_text ":97"
press Enter
type_text "x"
press Ctrl+X
pause 600
type_text "d"
pause 300
type_text ":96"
press Enter
type_text "P"
pause 1000
type_text "<"
pause 600
type_text "S"
pause 400
type_text "I"
type_text "let "
press Escape
pause 300
type_text "x"
type_text "s"
type_text ":"
press Enter
type_text "c"
type_text " ="
press Escape
pause 300
type_text "x"
type_text "s"
type_text ","
press Enter
type_text "c"
type_text ";"
press Escape
pause 600
type_text ":98"
press Enter
type_text "o"
type_text "x,"
press Enter
type_text "y,"
press Escape
pause 1000

# ── Scene 6 — step back through the session in the undo tree ─────────────────
# Enter moves the buffer to the highlighted revision and draws what that
# revision changed inline. The last jump returns to the newest revision.
type_text ":undotree"
press Enter
pause 2000
press Down 4
press Enter
pause 1500
press Down 2
press Enter
pause 1500
press Ctrl+U
press Enter
pause 800
press Escape
pause 800

# ── Scene 7 — the plugin's command on the manual's table, in two panes ───────
# The file picker opens demo.scm, held long enough to read: the split below
# leaves its pane too short for the whole command. Ctrl+S in a picker opens the
# pick in a split below. `m i p` selects the table, and :demo-table aligns it, redraws its
# separator row and frames it with a rule above and below.
type_text "zf"
pause 400
type_text "demo.scm"
pause 600
press Enter
pause 1200
type_text "zf"
pause 400
type_text "getting-started"
pause 800
press Ctrl+S
pause 1200
type_text ":36"
press Enter
pause 600
type_text "mip"
pause 1000
type_text ":demo-table"
press Enter
pause 3000
