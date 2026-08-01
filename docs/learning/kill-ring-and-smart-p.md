# Kill Ring and Smart-p: Two Sources of Paste

## The papercut HUME wants to avoid

In a register model with a single default destination — where every yank,
delete, and change writes to the same place, and bare paste reads from that
same place — routine editing constantly overwrites the text the user was
holding for paste. The user yanks a word, moves to fix something, deletes the
typo, and the original yank is gone. The workaround is to defensively name a
register for anything that should survive more than one operation.

This is a well-known friction point with Vim's traditional register model,
and HUME's first design decision is to not inherit it. HUME's primary capture
buffer is not a single slot but a **kill ring**: a bounded history of recent
captures, where a new delete adds an entry rather than overwriting the
previous one.

## How the kill ring works

The kill ring is a fixed-size queue of the last ten yanks, deletes, and
changes. Newest at the head; once full, the oldest entry falls off. Every
editing capture pushes a new entry — with one exception: if the current head
is a *pure whitespace* entry (spaces, tabs, newlines), the new entry overwrites
that slot in place rather than taking a fresh one. Deleting a space to fix a
typo (`dp`) should not cost you a ring slot you would never want to cycle back
to. The just-deleted whitespace stays retrievable until the next push, so the
swap itself still works; only afterwards is it gone. To keep whitespace durably,
yank it into a named register (`"0`–`"9`).

Ring entries are reachable in two ways: by pasting the head with `"kp` or by
relative position via `[`/`]` cycling (covered below). The ring keeps up to
ten entries.

**What the kill ring rescues you from:**

- Recovering text that was deleted a moment ago after some other operation
  interleaved. The deleted text lives at the ring head until something newer
  pushes it down; it is reachable via `"kp` or by cycling.
- Reaching one of several recent deletes without having captured each into a
  named register up front. The ring provides up to ten entries of
  backward-looking insurance.
- Assembling a paste from disparate earlier captures by cycling through
  recent history instead of typing register names.

That alone — without any system-clipboard integration — already fixes the
papercut: a delete cannot destroy a previous capture, only add a new one to
the ring.

## One-keystroke yank to the system clipboard

The other thing every editor user expects is friction-free export to the OS
clipboard. Yank in the editor, paste in a browser. If yank-to-clipboard
required a register prefix every time, it would be a constant tax on a
workflow that should be free.

So bare yank writes to **both** the system clipboard and the kill ring. Bare
delete and change write only to the kill ring — the clipboard is never
touched by routine editing. Together with the previous section, this gives
two protections at once:

- Cross-application paste is one keystroke.
- An accidental delete cannot clobber what the user copied from another
  application a moment ago.

## Smart-p: the heuristic

When the user presses `p` for a bare paste, which source does it read — the
clipboard, or the ring head?

HUME answers by asking a narrower question: *has anything been edited since
the ring last changed?* If not, paste reads the ring — the buffer is still in
the state the capture left it in, so the ring head is still "the thing that
was just here." If something has been edited since, paste reads the system
clipboard instead — the user has moved on, and cross-app paste stays the
default.

- **Nothing edited since the last capture (`d`/`c`/`y`)** → paste reads the
  kill-ring head. Swap idioms work: `d`, move around, `p` restores what was
  killed, as long as no edit happened along the way.
- **Something edited since** → paste reads the system clipboard, falling back
  to the ring head if no OS clipboard is available (a headless environment).

Motions, searches, and undo/redo don't fit either bucket the way a command
name would — they're judged by what they actually do to the buffer. A motion
touches nothing, so it never switches the source; undo and redo *are* edits
(they change what's on screen), so they do switch it. This replaces the older
design, which routed by matching the *name* of the previous command against a
fixed allow-list (`change`, `delete`) — that list needed hand-maintained
exceptions for `exit-insert` (to not break the swap idiom on `<esc>`) and for
macro replay and dot-repeat (both forced to the clipboard unconditionally, to
keep replay deterministic). Judging by buffer state instead of command name
needs none of that: dot-repeating a delete is itself a fresh capture, so a
paste right after `.` correctly reads what `.` just deleted — no special case
required.

**Repeat presses still work like a stack.** `xd p p p` produces three copies.
Each press re-reads the same source fresh; since nothing has changed the
ring (or clipboard) between presses, the same value comes back each time —
and pasting a value that matches what's already selected appends alongside it
rather than replacing it (see "Repeat vs. swap" below), so the copies stack.

## Repeat vs. swap: smart-paste's one rule, not two mechanisms

Two things used to feel like separate smart-paste behaviors — "paste over a
selection replaces it" and "pasting again after a paste appends another
copy" — but they're the same rule seen from different starting points: **if
what you're about to paste is already exactly what's selected, collapse the
selection and paste next to it instead of over it.**

- Selection is a bare cursor → there's nothing to compare against, so paste
  just lands next to the cursor.
- Selection holds different text than what's being pasted → replace it. This
  is the ordinary case: select a word, paste a replacement over it.
- Selection holds the *same* text that's about to be pasted → collapse first,
  then paste next to it. This is what makes `p p p` stack: after the first
  paste, the pasted text is selected, and the second paste's source resolves
  to that same text again, so it appends rather than clobbering the first
  copy.

This is smart-paste-only. Plain paste never compares — see "Plain paste
versus smart paste" below for why.

## Plain paste versus smart paste

Everything above describes *smart* paste — the heuristic bound to the
default paste keys, in both what it reads (ring vs. clipboard) and how a
repeat behaves (append vs. replace, per "Repeat vs. swap" above). HUME also
exposes a *plain* paste with none of that: it always reads the kill-ring
head, with no clipboard fallback, and it always replaces a non-collapsed
selection outright — no text comparison, ever. Pasting the same text twice
over the same selection just replaces it with itself the second time; to
stack a copy, collapse the selection first (`;`), then paste.

That "no comparison, ever" is deliberate, not an oversight: plain paste's
whole reason to exist is predictability for code driving it programmatically
— a keymap or a plugin script that selects some text and pastes should never
have to first ask "is this exactly what's already there?" to know what will
happen. Smart-paste's repeat-vs-swap rule exists because a human pressing `p`
twice in a row has an intent worth inferring; a script pasting over a
selection has already stated its intent by choosing that selection, and
inferring around it would make plain paste unpredictable for the one use
case it exists for.

Plain paste has no key bound by default; the default keys run the smart
variant. A bundled alternate keymap plugin rebinds the default paste keys to
the plain commands instead, pairing them with the system clipboard on
separate keys — trading Smart-p's single "just paste" key for two dedicated,
always-predictable ones.

## Paste sessions and cycling

When the ring head isn't the entry you wanted, `[` and `]` step through the
ring without typing a slot name. Cycling in HUME uses a **paste session**
model:

- The first `p` (or `P`) opens a **paste session**: it snapshots the
  pre-paste buffer state, pastes the chosen entry, and leaves the session
  open. The pasted text is selected.
- While the session is open, `[` replaces the paste with the next-older
  entry and `]` replaces it with the next-newer one. Each cycle re-pastes
  from the same pristine snapshot in the same direction as the opening `p`/`P`
  — so `P [ [` pastes above the cursor just as `P` itself did; `p [ [`
  pastes after. The previous paste's effect is cleanly discarded — no stray
  lines accumulate.
- The entire session (the initial paste plus all cycles) records as **one
  undo step**. `p` then many `[`/`]` then `u` restores the buffer to its
  state before the first `p`.
- The session commits when any command other than `[` or `]` fires. The
  next `p` opens a fresh session.

**Stepping clamps at the ends of the ring** rather than wrapping. If you keep
pressing `[` and reach the oldest entry, further presses do nothing. Clamping
makes the boundary apparent; wrapping would hide it.

**Cycling can't return to a clipboard paste.** When the session was opened by a
clipboard paste — bare `p` with no preceding delete or change — the clipboard
text is the *opener*, not a ring entry. The first `[` steps into the ring at
its head; from there `[` and `]` move among ring entries only. Because the
clipboard value has no place in the ring, `]` clamps at the ring head and can
never step back out to the original clipboard paste. To paste the clipboard
again, start a fresh paste with the explicit clipboard register (`"cp`). This
falls out of the model directly: the cycle position is a position within the
ring's history, and the clipboard lives outside that history.

**`[` and `]` without an open session are noops.** To cycle, you must first
paste with `p` or `P`.

## Linewise versus charwise paste

A register's content chooses its own paste shape from how it ends. Content
ending in a newline is *linewise*: over a cursor it inserts as new line(s)
below or above the cursor line; over an explicit selection it replaces
line-by-line and reflows what is left. Content without a trailing newline is
*charwise*: it lands inline at the cursor, or replaces the selected span in
place. The distinction falls out of inspecting the yanked text — no separate
`P`-linewise command names it, and cycling the kill ring moves freely between
linewise and charwise entries as the cycle position changes.

**Consecutive `p` presses append** rather than replacing — see "Repeat vs.
swap" above for the underlying rule. Each press is its own separate undo
step, distinct from a `[`/`]` cycle (which folds into the one undo step the
paste session opened).

**`p` after `[`/`]` appends the cycled entry.** If you cycled to the
second-oldest entry and then press `p`, the new paste appends a second copy
of that entry (not the head), and the cycle position is preserved so a
following `[` continues from there rather than resetting.

## Explicit register prefix is untouched

Everything above describes the *bare* yank, delete, change, and paste keys.
Prefixing any of them with a register name bypasses the heuristic:

| Prefix | Behaviour |
|--------|-----------|
| `"c` | System clipboard: yank writes clipboard only (no ring push); paste always reads clipboard |
| `"k` | Kill-ring head: paste reads the most-recent ring entry; yank/delete/change push onto the ring without touching the clipboard. Older entries reachable via `[`/`]`. |
| `"0`–`"9` | Symmetric in-memory storage: `"5y` and `"5p` both use the same named slot, so they round-trip. No kill-ring interaction. The same slot also stores macros recorded with `Q5`/`q5` — last write wins. |
| `"b` | Black hole: yank discards; paste reads nothing |

The two non-clipboard register namespaces serve different purposes. The kill
ring (`"k`, `[`/`]`) is for *interactive* use: the ring head is non-deterministic
— every `d`/`c`/`y` shifts it — so it is reach-back for in-the-moment editing, not
durable storage. The digit slots (`"0`–`"9`) are for *scripted, surgical, or
programmable* use: deterministic named slots that round-trip (`"5y` then `"5p`
returns exactly what was written, regardless of intervening edits); the same slots
double as macro registers (`Q5`/`q5`).

Smart-p is a default for the bare keys, not a constraint on the register
system. If the heuristic ever routes to the wrong source, the register
prefix is the explicit override.

## Comparison with Vim

Vim addresses parts of the same tension, but with different tools. The
contrast is worth tracing because it shows where HUME's choices come from.

**Single default register.** Vim's unnamed register (`"`) receives most
yanks and deletes, and bare paste reads it. The classic friction sequence
is `yiw`, navigate, `dd`, `p`: the `p` pastes the deleted line, not the
yanked word, because the `dd` clobbered the unnamed register. Vim's partial
fix is the dedicated yank-only register `"0`, which captures yanks but
never deletes — so the workaround is `yiw`, navigate, `dd`, `"0p`. The
papercut is real enough that "use `"0p` after a delete-then-yank" is common
folklore. In HUME, `"kp` is the explicit kill-ring-head paste, and Smart-p
handles the common case automatically.

**System clipboard as a separate register.** The OS clipboard sits behind
`"+` (and `"*` on macOS). By default `y` and `p` ignore it; cross-app paste
costs a prefix every time (`"+y`, `"+p`). The escape hatch is
`set clipboard=unnamedplus` in `.vimrc`, which makes the unnamed register
*be* the clipboard. That gets one-key cross-app paste, but at the cost of
making every delete clobber the system clipboard — the same single-register
papercut, propagated outward.

**No first-class history.** Vim has undo and `:earlier`, but no bounded
ring of recent captures. Recovering "the line I deleted three operations
ago" requires having yanked it into a named register at the time, or
walking the undo tree.

HUME's design splits the destinations instead of layering more registers
on top of one. Bare yank goes to the clipboard *and* the ring; bare delete
and change go to the ring only; bare paste consults Smart-p to decide
which source to read. Cross-app paste is one keystroke without
`unnamedplus`-style trade-offs, deletes never reach the clipboard, and the
ring keeps the last ten captures alive. The goal overlaps with Vim's; the
mechanism is different.

---

*See also: [Edit Operations](edit-operations.md) for the register table and the
select-then-act model underlying how paste interacts with selections.*
