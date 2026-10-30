# Registers, the Kill Ring, and Smart-p

## The papercut HUME avoids

In a register model with a single default destination, every yank, delete,
and change writes to the same place, and bare paste reads from it. Routine
editing then keeps overwriting the text the user was holding for paste: yank
a word, move to fix a typo, delete it, and the yank is gone. The workaround
is to name a register for anything that should survive more than one
operation.

HUME splits the destinations instead. Captures go to a **kill ring**, a short
history where a new delete adds an entry rather than overwriting the last
one. A bare yank also goes to the system clipboard, and a bare paste decides
which of the two to read.

## Registers

Registers have one-character names. Digits hold user storage, which leaves
letters free for the special registers:

| Register | `"` prefix | Behaviour |
|----------|:----------:|-----------|
| `0`–`9` | yes | In-memory storage for text or macros, last write wins. `"5y` and `"5p` use the same slot, so they round-trip. No kill-ring interaction. |
| `k` | yes | Kill ring. Paste reads the head; yank, delete, and change push onto the ring without touching the clipboard. |
| `c` | yes | System clipboard. Yank writes the clipboard only (no ring push); paste always reads it. |
| `b` | yes | Black hole. Writes are discarded; paste reads nothing. |
| `q` | no | Default macro register: `QQ` records into it, `qq` replays it. |
| `s` | no | The last search pattern, written by search. |

A register prefix bypasses everything the rest of this page describes about
bare keys. If smart paste ever reads the wrong source, the prefix is the
explicit override.

Macros live in the same digit slots as text: `Q5` records into `5` and `q5`
replays it, and a later `"5y` overwrites the macro. Keeping macros in named
slots lets two macros coexist without re-recording either.

The digits and the kill ring serve different purposes. The digits are
deterministic: what `"5y` wrote, `"5p` reads back, whatever happened in
between, so they suit macros and scripted or careful use. The ring head moves
with every `d`, `c`, and `y`, so the ring is for reaching back during
interactive editing, not for storage.

## How the kill ring works

The kill ring holds the last ten yanks, deletes, and changes, newest at the
head. Once it is full, the oldest entry falls off. Every capture pushes a new
entry, with two exceptions.

If the head is pure whitespace (spaces, tabs, newlines), the new entry
overwrites that slot instead of taking a fresh one. Deleting a stray space
should not cost a ring slot nobody would cycle back to. The whitespace stays
retrievable until the next push, so a swap still works. To keep whitespace
durably, yank it into a digit register.

If the new entry matches one already in the ring, that entry moves to the
head instead of taking a fresh slot. Two entries match when their text and
their paste shape (see [Linewise versus charwise
paste](#linewise-versus-charwise-paste)) are both equal. Killing `foo` in two
places leaves one `foo` in the ring, so cycling with `[`/`]` never lands on an
entry already seen. The same text captured once as lines and once as
characters is two entries, since the two paste differently.

The ring rescues text deleted a moment ago after other operations
interleaved, reaches one of several recent deletes without having named a
register up front, and lets a paste be assembled from earlier captures by
cycling. A delete cannot destroy an earlier capture, only add a new one.

## One-keystroke yank to the system clipboard

Copying to another application should cost nothing extra: yank in the
editor, paste in a browser. So a bare yank writes to **both** the system
clipboard and the kill ring. A bare delete or change writes only to the
ring. Cross-application copy is one keystroke, and an accidental delete
cannot clobber what the user just copied from another application.

## Smart-p: the heuristic

A bare `p` has two possible sources: the clipboard or the ring head. HUME
picks by asking whether anything was edited since the ring last changed.

- **Nothing edited since the last `d`, `c`, or `y`**: paste reads the ring
  head. The buffer is still in the state the capture left, so the head is
  still "the thing that was just here". Swap idioms work: `d`, move, `p`
  restores what was killed.
- **Something edited since**: paste reads the system clipboard. The user has
  moved on, and cross-application paste stays the default. With no OS
  clipboard (a headless environment), a fresh paste falls back to the ring
  head. A repeat press whose clipboard cannot be read does nothing instead,
  because substituting the ring head would replace the text the previous
  press just pasted.

The rule judges the buffer, not the command that ran. A motion changes no
text, so it never switches the source. Undo and redo change the text, so
they do. There is no list of "kill-like" commands to maintain, and macro
replay and dot-repeat need no special case: repeating a delete is itself a
fresh capture, so a paste right after `.` reads what `.` deleted.

Repeat presses stack copies: `xd p p p` produces three. Each press reads the
same source again, and the next section explains why the copies land side by
side.

## Repeat vs. swap

"Paste over a selection replaces it" and "pasting again appends another copy"
are one rule seen from two starting points: **if the text about to be pasted
is already what is selected, collapse the selection and paste next to it
instead of over it.**

- The selection is a bare cursor: there is nothing to compare, so the paste
  lands next to the cursor.
- The selection holds different text: the paste replaces it. This is the
  ordinary case of pasting a replacement over a selected word.
- The selection holds the same text: collapse, then paste next to it. After
  the first paste the pasted text is selected, and the second press resolves
  to that same text, so it appends instead of replacing the first copy.

Text displaced by a replacing paste is not written back to any register. The
ring already holds the history of what was selected, so `"kp` or `[`/`]`
reaches it.

## Plain paste versus smart paste

Smart paste is what the default paste keys run: it chooses ring or
clipboard, and it applies repeat vs. swap. HUME also has a *plain* paste with
neither rule. It always reads the ring head and always replaces a selection
of more than one character, without comparing text. Pasting the same text
twice over the same selection replaces it with itself; to stack a copy,
collapse the selection first (`;`).

Plain paste exists for code that drives the editor. A keymap or a script
that selects text and pastes has already stated its intent by choosing the
selection, and should never need to ask whether the text is already there to
know what will happen. A person pressing `p` twice has an intent worth
inferring; a script does not.

Plain paste has no default key. A bundled alternate keymap plugin rebinds the
paste keys to the plain commands and puts the system clipboard on separate
keys, trading one "just paste" key for two predictable ones.

## Paste sessions and cycling

When the ring head is not the entry you wanted, `[` and `]` step through the
ring without typing a register name. Cycling uses a **paste session**:

- The first `p` (or `P`) opens a session. It snapshots the buffer, pastes the
  chosen entry, and selects the pasted text.
- While the session is open, `[` replaces the paste with the next-older entry
  and `]` with the next-newer one. Each step re-pastes from the same snapshot
  in the direction of the opening key, so `P [ [` still pastes above the
  cursor. No stray lines accumulate.
- The whole session, the first paste and every step, is **one undo step**.
  `p`, many `[`/`]`, then `u` restores the buffer to its state before the
  `p`.
- Any command other than `[` or `]` closes the session. The next `p` opens a
  fresh one.

Stepping **clamps** at both ends of the ring instead of wrapping, so the
boundary is visible: at the oldest entry, further `[` presses do nothing.

A `p` right after a paste appends another copy, as repeat vs. swap describes,
and each such press is its own undo step. After cycling, `p` appends a copy
of the entry cycled to, not the head, and keeps the cycle position, so a
following `[` continues from there.

**Cycling cannot return to a clipboard paste.** When bare `p` read the
clipboard, the clipboard text opened the session but is not a ring entry.
The first `[` steps into the ring at its head, or at the entry after it when
the head already holds the pasted text (the usual case after `y`, which
writes both). From there `[` and `]` move among ring entries only. To paste
the clipboard again, use `"cp`.

`[` and `]` without an open session do nothing.

## Linewise versus charwise paste

Every register entry remembers the paste shape it was captured with:

- A `d` or `y` of whole lines (the selection starts a line and ends on a line
  break) is *linewise*. Over a cursor it inserts as new lines below (`p`) or
  above (`P`) the cursor's line. Over a selection it replaces line by line
  and reflows what is left.
- Anything else `d` or `y` captures is *charwise*: it lands inline at the
  cursor, or replaces the selected span.
- `c` always captures *charwise*. It removes a selection's content but keeps
  the line break the selection ends on, so it rewrites a line without
  removing it.

The shape comes from the capture, not from inspecting the text afterwards.
Text that did not come from a selection, such as the system clipboard or text
a script wrote, has no remembered shape. It counts as linewise when it ends
in a newline, since that is the only signal it carries. Cycling the ring
moves freely between linewise and charwise entries.

When a linewise paste replaces a selection that does not start its line, the
text before the selection moves onto a line of its own, so the pasted lines
start a line. When the selection ends right before its line's break, the
paste takes that break too, so no blank line is left behind.

`d` and `y` share one rule for what a selection captures, so a yank holds
the same text a delete of the same selection would remove. Whole lines are
captured with their line breaks. Anything else is captured short of the
buffer's structural final line break, which no edit removes. A selection
covering only that final break captures nothing, and `d` removes nothing
from it.

## Several entries, a different number of cursors

A yank from several selections captures one entry per selection, in
document order. Pasting with as many cursors as entries gives each cursor its
own entry, the first entry to the first cursor. With any other number of
cursors, every cursor gets all the entries, joined in order and pasted in the
shape of the last one. A single entry is the simplest case of the same rule:
every cursor gets it.

Joining adds nothing between entries. Whole lines each end in their own line
break, so they still paste as lines. Characters run together, so `foo` and
`bar` pasted at one cursor give `foobar`.

Kakoune and Helix agree with HUME when the counts match, and differ when they
don't:

| Entries → cursors | Kakoune | Helix | HUME |
|---|---|---|---|
| 1 → any | every cursor gets it | every cursor gets it | every cursor gets it |
| N → N | one each | one each | one each |
| N → 1 | the first entry only | the first entry only | all of them |
| N → more than N | entries start again from the first | the last entry repeats | all of them at every cursor |
| N → fewer than N | the first ones, the rest unused | the first ones, the rest unused | all of them at every cursor |

Joining on a mismatch follows what users of both editors report:

- **Several entries into one cursor should paste them all.** Getting only
  the first entry is the complaint that recurs in both trackers
  ([kakoune#829], [helix#14039]). Both projects answer with a separate
  command that pastes every entry: Kakoune has `<a-p>`, and the same request
  for Helix is still open ([helix#1333], [helix PR #13583]). HUME's ordinary
  paste covers that case without a second key.
- **A wrong count should show.** Repeating or wrapping hands every cursor a
  plausible entry, so a miscount goes unnoticed. A joined paste is visibly
  wrong and one undo away. A Helix user makes the same point in
  [helix discussion #10200]: "with the current Helix behavior it's harder to
  notice the mistake."
- **No one asks for repeating or wrapping.** The author of [helix#7792]
  writes "I can't really think of a situation where I'd want Helix to repeat
  the last selection multiple times", and proposes the rule HUME uses, which
  they attribute to VS Code and Sublime. Helix's maintainers keep the repeat
  as intended behaviour ([helix#14039]). None of these threads asks for
  Kakoune's wrapping. What it enables, such as alternating two entries down a
  column, turns up in keystroke-golf solutions rather than in requests.

[kakoune#829]: https://github.com/mawww/kakoune/issues/829
[helix#1333]: https://github.com/helix-editor/helix/issues/1333
[helix#7792]: https://github.com/helix-editor/helix/issues/7792
[helix#14039]: https://github.com/helix-editor/helix/issues/14039
[helix PR #13583]: https://github.com/helix-editor/helix/pull/13583
[helix discussion #10200]: https://github.com/helix-editor/helix/discussions/10200

---

*See also: [Edit Operations](edit-operations.md) for the select-then-act
model that paste builds on.*
