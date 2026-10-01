# Buffer Invariants and Plugin Safety

## The invariants

Every buffer in HUME must satisfy three invariants at all times:

1. **Trailing newline**: the buffer always ends with a newline character. This
   is the "structural newline" — it guarantees every line has a terminator and
   means a cursor can always sit on a valid character. Without it, the very
   last position in the file would be undefined, and every line-iteration
   algorithm would need a special case.

2. **Non-empty**: at least one character must exist. This follows from the
   trailing newline but is worth naming explicitly because several algorithms
   assume it.

3. **No raw carriage returns**: whatever line-ending convention a file, a
   paste, or a language server's edit used on the way in, the buffer itself
   only ever holds plain `\n`. A Windows-style pair and an old-Mac-style lone
   carriage return both collapse to `\n` before the text becomes buffer
   content. This means every line-splitting algorithm in the editor has
   exactly one terminator to look for, never a family of them — and a
   separate per-buffer flag remembers which convention the file used, purely
   so saving can write it back the way it came. That flag describes the file
   on disk; it never describes what the buffer holds in memory.

Every selection must also satisfy:

4. **Whole characters, in bounds**: each end of a selection is the start of a
   character cluster (a character as the user sees it, such as a letter with
   its accent or an emoji sequence) that lies below the buffer's end. A
   selection cannot point past the buffer or into the middle of a cluster.
   Because the structural newline is always there, a cursor on the last line
   always has a character to sit on.

5. **A well-formed set**: a set of selections is never empty, is sorted by
   position, and no two selections share a cluster. Selections that would
   share one are merged into one.

6. **A version tag**: a set of selections carries the version of the text it
   was computed for, and can only be read against that text. Pairing a set
   with another text is a bug in whoever paired them, and it panics instead of
   acting on the wrong text.

## One rule for the final newline

Suppose an edit would delete the trailing newline: a delete-to-end command, or
a language server whose file has no final newline replacing the whole text.
A changeset by itself would produce a buffer without its structural newline,
and applying it directly is an error. Edits never reach that error, because
every edit is built under one rule:

- When the edit would leave the buffer without a final newline, text inserted
  at the end of the buffer gets a newline of its own.
- When nothing is inserted at the end, every deletion stops before the
  structural newline.

The rule lives in one place and every source of edits goes through it,
language-server edits included. A server's edits describe a file that may not
end with a newline, and the buffer gets one back.

The result is neither a repair after the fact nor a crash. A repair that appends a
character after the changeset is built would leave the changeset's declared
resulting length off by one, and composing changesets or inverting an edit for
undo would then use the wrong length. The bug would surface far from where the
repair happened. The rule avoids that by shaping the changeset itself, so the
changeset describes the text the buffer really ends up with.

## Where to check

There are two kinds of call sites:

- **Internal commands** (character insertion and deletion, motion code): these
  build their edits with the edit builder, which only accepts whole character
  clusters and applies the final-newline rule, so they cannot violate the
  invariants. A broken internal edit is an engine bug, and a hard crash with a
  diagnostic message is appropriate.

- **The trust boundary**: a script can submit a raw edit directly, most
  commonly for a language-server rename or formatting pass. That path is
  validated up front: is the buffer writable? does it still match the buffer
  generation the edit was computed against? are the ranges well-formed and
  non-overlapping? Only then is a changeset built, under the final-newline
  rule. The editor derives the resulting cursor positions itself by mapping the
  existing selections through the edit, so no untrusted selection ever enters.
  Most plugin code never touches even that path; it issues named editor
  commands instead, each of which constructs its edit internally.

History replay needs no validation of its own. Every undo and redo replays
changesets the editor itself recorded, so a replay that fails means the history
is corrupt, and the editor stops with a diagnostic instead of miscomputing.

There is one other place untrusted text enters the buffer: reloading a file
from disk (`:e!`). The contents come from outside the editor, but they enter
as a forward and inverse changeset derived from a line-level diff against the
current buffer, not as raw text replacing the buffer. The algebra still
applies: a reload whose net effect equals what's already on disk records no
revision at all.

Adding validation to every internal function would be noise — forcing internal
code to handle errors that provably cannot occur. The right design is:
**validate once at the boundary, trust everything inside**. The boundary is
narrow and well-defined.

During development, internal code uses lightweight assertions that only run in
debug builds. These assertions catch engine bugs during testing without paying
any cost in release builds.

## Making the violation unreachable

Repairing, crashing, and rejecting all assume the invariant is checked *after*
a value already exists. The no-raw-carriage-return invariant takes a different
approach, because checking it after the fact is expensive: the trailing-newline
and in-bounds invariants can be checked in the time it takes to look at one
position, but a stray `\r` could be anywhere in an arbitrarily large buffer,
so confirming its absence means scanning every character.

Instead of validating that expensive property on every edit, HUME makes it
impossible to construct a violation at all. There are exactly two places
where text from outside the editor becomes buffer content, and both convert
line endings to `\n` before the text goes anywhere else. Every piece of code
downstream of those two points can simply assume the property holds, the
same way it assumes the trailing newline holds — the difference is *how* the
guarantee is produced: at a narrow choke point where the text is built,
rather than by checking a value that already exists. A lightweight debug-only assertion
still confirms the invariant holds, purely to catch a bug in the editor
itself — it plays no role in maintaining the guarantee for ordinary use.

## Reverting on failure without explicit cleanup

Consider the sequence: build an inverse changeset (for undo), apply the
forward changeset, check the result. If applying fails, the forward changeset
is rejected, but the inverse is already built. Does it need cleanup?

No. The inverse is just a value on the stack. When the failure branch is taken
and execution leaves that scope, the value is automatically freed. There is
nothing to clean up.

This is a small example of how Rust's ownership model turns resource management
into a mechanical property of the language rather than a manual obligation.
Allocating a temporary, using it on the success path, and automatically
discarding it on the failure path requires zero explicit cleanup code.

```rust
let inverse = changeset.invert(&original);   // build while original is intact
match changeset.apply(&original) {
    Ok(new_text) => { /* push inverse onto the undo tree */ }
    Err(reason)  => { /* inverse is freed here, no cleanup needed */ }
}
```

## Why applying an edit takes the text by reference

Applying a changeset takes the text by reference, not by value. Taking it by
value would let the underlying rope be mutated in place, but if the apply then
failed, the text would be gone and the caller could not recover the original.

By reference, applying clones the rope first. Cloning costs almost nothing
because the rope uses arc-based structural sharing: cloning just bumps a
reference count, sharing the whole tree. Applying then works on the clone,
checks the post-conditions, and only wraps the clone in a new text if
everything succeeded. On failure, the clone is dropped and the original is
intact.

The key insight is that "recoverable failure" and "mutation in place" are
in tension. Taking a reference and cloning a reference-counted tree is the
pragmatic middle ground.

## Pairing a set of selections with its text

A set of selections is read only through a pairing with its text, and the
pairing is the one place that checks the two belong together. A mismatch is not
an error to recover from: it means some code kept a set across a text change
without carrying it through, so the pairing panics. Release builds always check
that no selection reaches past the end of the text, and debug builds check
every other invariant of the set.

The version tag on a set is the only guard against a set that outlived its
text, which is why every stored position is either carried through each change
or tagged so that it reads as absent once the text moves. See
[Changesets](changesets.md).
