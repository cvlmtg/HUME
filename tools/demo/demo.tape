# HUME demo recording — see tools/demo/README.md for the assets this drives.
#
# Record with tools/demo/record.sh from the repo root, which also drops blank
# frames from the GIF. The recording edits real files of this repo
# (hume-grid/src/geometry.rs and user-manual/docs/getting-started.md) and
# quits with :qa!, so nothing is saved.
#
# Story: jump across the workspace with the LSP symbol pickers, add a method
# with auto-pairs, auto-indent and a completion with its documentation box,
# wrap words in backticks with surround, toggle a comment, rename with sift,
# hoist two lines with a few multi-cursor edits, and step back through the
# session in the undo tree. Then open the Steel plugin through the file picker,
# the manual page in a split below it, and run the plugin's command on the
# manual's table.

Output tools/demo/demo.gif

Set Shell "bash"
Set FontSize 18
Set Width 1200
Set Height 720
Set Padding 0
Set BorderRadius 8
Set WindowBar Colorful
Set TypingSpeed 90ms
Set Theme "GruvboxDark"
Set Framerate 25

# ── Launch (off-camera) ───────────────────────────────────────────────────────
# rust-analyzer indexes the whole workspace before a workspace symbol query
# answers; the results list geometry.rs once it has. A query that arrives too
# early times out, so the first two attempts accept the timeout message and the
# third must list geometry.rs.
Hide
Type "./target/release/hume --config tools/demo/init.scm hume-grid/src/lib.rs"
Enter
Sleep 20000ms
Type "zS"
Type "Rect"
Wait+Screen@60s /geometry\.rs|timed out/
Escape
Sleep 15000ms
Type "zS"
Type "Rect"
Wait+Screen@60s /geometry\.rs|timed out/
Escape
Sleep 15000ms
Type "zS"
Type "Rect"
Wait+Screen@180s /geometry\.rs/
Escape
Sleep 1200ms
Show

# ── Scene 1 — workspace and document symbol pickers, inlay hints, hover ──────
# The workspace picker lists matches alphabetically, so Rect's definition in
# geometry.rs is the eighth row; seven Downs reach it.
Sleep 1200ms
Type "zS"
Sleep 400ms
Type "Rect"
Wait+Screen@20s /geometry\.rs/
Sleep 600ms
Down
Down
Down
Down
Down
Down
Down
Sleep 300ms
Enter
Sleep 800ms
Type "zs"
Sleep 400ms
Type "cent"
Sleep 1200ms
Enter
Sleep 1500ms
Type "/min"
Enter
Sleep 300ms
Type "K"
Sleep 300ms
Ctrl+D
Sleep 300ms
Ctrl+D
Sleep 300ms
Ctrl+D
Sleep 300ms
Escape
Sleep 400ms

# ── Scene 2 — a new method: auto-pairs, auto-indent and a completion ─────────
# `.` is a trigger character for rust-analyzer, so the menu opens without a
# key; `ating_su` narrows it to saturating_sub, whose docs show in the box
# beside the menu. Accepting inserts the bare name, without parens. The first
# Escape closes the signature popup, the second leaves Insert mode.
Type ":102"
Enter
Type "o"
Enter
Type@45ms "/// Removes rows from the bottom of self, saturating at a height of zero."
Enter
Type@45ms "pub fn without_bottom(self, rows: u16) -> Rect {"
Enter
Type@45ms "Rect {"
Enter
Type@60ms "height: self.height.satur"
Sleep 600ms
Type "ating_su"
Sleep 600ms
Ctrl+D
Sleep 1200ms
Enter
Type "(rows),"
Enter
Type "..self"
Sleep 900ms
Escape
Sleep 300ms
Escape
Sleep 500ms

# ── Scene 3 — surround: backticks around three words at once ─────────────────
# `s` sifts the doc line down to one cursor per match; `m w` adds the pair the
# next key names.
Type ":104"
Enter
Type "x"
Type "s"
Type '\b(rows|self|height)\b'
Sleep 500ms
Enter
Sleep 700ms
Type "mw"
Type '`'
Sleep 1200ms

# `G G` toggles a comment on the selected function; a second press undoes it.
Type ":106"
Enter
Type "maf"
Sleep 600ms
Type "GG"
Sleep 600ms
Type "GG"
Sleep 300ms
Type ";"

# ── Scene 4 — document symbols, then rename with sift ────────────────────────
# `m i p` selects the doc comment and the function together, so the doc's
# mention of the parameter is renamed with the six others. Ctrl+O returns to
# the previous jump.
Type "zs"
Sleep 400ms
Type "contains"
Sleep 1000ms
Enter
Sleep 800ms
Type "mip"
Sleep 400ms
Type "s"
Type '\bpos\b'
Sleep 500ms
Enter
Sleep 800ms
Type "c"
Type "cell"
Escape
Sleep 800ms
Type "u"
Sleep 400ms
Ctrl+O
Ctrl+O
Sleep 800ms

# ── Scene 5 — move lines and turn them into bindings ─────────────────────────
# `x` then Ctrl+X selects two lines. They paste above `Rect {` one level too
# deep, and `<` dedents the pasted selection. `S` splits it into one cursor per
# line, `I` inserts at each line's start, and `x` re-selects each cursor's line
# for the next sift. The struct literal then takes shorthand fields.
Type ":97"
Enter
Type "x"
Ctrl+X
Sleep 600ms
Type "d"
Sleep 300ms
Type ":96"
Enter
Type "P"
Sleep 1000ms
Type "<"
Sleep 600ms
Type "S"
Sleep 400ms
Type "I"
Type "let "
Escape
Sleep 300ms
Type "x"
Type "s"
Type ":"
Enter
Type "c"
Type " ="
Escape
Sleep 300ms
Type "x"
Type "s"
Type ","
Enter
Type "c"
Type ";"
Escape
Sleep 600ms
Type ":98"
Enter
Type "o"
Type "x,"
Enter
Type "y,"
Escape
Sleep 1000ms

# ── Scene 6 — step back through the session in the undo tree ─────────────────
# Enter moves the buffer to the highlighted revision and draws what that
# revision changed inline. The last jump returns to the newest revision.
Type ":undotree"
Enter
Sleep 2000ms
Down
Down
Down
Down
Enter
Sleep 1500ms
Down
Down
Enter
Sleep 1500ms
Ctrl+U
Enter
Sleep 800ms
Escape
Sleep 800ms

# ── Scene 7 — the plugin's command on the manual's table, in two panes ───────
# The file picker opens demo.scm, held long enough to read: the split below
# leaves its pane too short for the whole command. Ctrl+S in a picker opens the
# pick in a split below. `m i p` selects the table, and :demo-table aligns it, redraws its
# separator row and frames it with a rule above and below.
Type "zf"
Sleep 400ms
Type "demo.scm"
Sleep 600ms
Enter
Sleep 1200ms
Type "zf"
Sleep 400ms
Type "getting-started"
Sleep 800ms
Ctrl+S
Sleep 1200ms
Type ":36"
Enter
Sleep 600ms
Type "mip"
Sleep 1000ms
Type ":demo-table"
Enter
Sleep 3000ms

# ── Done ───────────────────────────────────────────────────────────────────────
Type ":qa!"
Enter
