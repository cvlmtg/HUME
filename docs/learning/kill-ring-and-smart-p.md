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
editing capture pushes a new entry; nothing in the ring is overwritten in
place.

Ring entries are reachable in two ways: by pasting the head with `"kp` or by
relative position via `[`/`]` cycling (covered below). The ring keeps up to
ten entries; nothing is overwritten in place.

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

HUME records the name of the most recent command. When bare paste fires, it
reads that name and routes accordingly:

- **Previous command was a delete or change** → paste reads the kill-ring head
  (most recently killed text). Swap idioms work.
- **Previous command was a paste or paste-cycle** → consecutive paste presses
  append from `last_paste` (the same values verbatim), independent of what the
  ring or clipboard hold at that moment.
- **Anything else** → paste reads the system clipboard. Cross-application
  paste remains one keystroke.

The intuition behind the allow-list (this short set keeps paste in the ring;
everything else switches to clipboard):

- The user can apply one rule without memorising special cases: *"did I just
  delete or change? Then the next paste reads the ring. Otherwise the
  clipboard."*
- Motions, searches, and undo all clear the path. These operations typically
  signal a context switch — the user has moved on from "edit this spot" to
  something else. A paste at the new location should usually be the
  cross-app one.

A few non-obvious entries are worth calling out explicitly:

**Paste itself keeps the ring active.** Consecutive paste presses all read
the ring head. You can paste the same deleted text multiple times — `xd p p
p` produces three copies — without the second or third paste silently
switching to the clipboard.

**Yank is not in the allow-list.** A yank is the moment the user has chosen
to capture text for export — the next paste should hit the clipboard, not
the ring. The yanked text is still pushed into the ring and remains
reachable by name or by cycling; it is simply not the first thing bare paste
reaches for.

**Macro replay is a single non-exception.** After a macro replays, the
heuristic is left in a "not delete, not change" state regardless of what the
macro contained. This makes replay deterministic — the paste source after
`q<reg>` is always the clipboard, independent of the macro's contents — and
avoids surprising flip-flops inside replay loops.

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

**`[` and `]` without an open session are noops.** To cycle, you must first
paste with `p` or `P`.

**Consecutive `p` presses append** rather than replacing. After a paste, the
pasted text is selected. A second `p` collapses that selection and pastes
again adjacent to it — two copies side by side, each a separate undo step.
This is intentional: the session model makes cycle-replace cheap (`[`/`]`),
so consecutive `p` can safely mean "add another copy".

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
