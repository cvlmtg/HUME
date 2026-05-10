# Buffer Invariants and Plugin Safety

## The invariants

Every buffer in HUME must satisfy two invariants at all times:

1. **Trailing newline**: the buffer always ends with a newline character. This
   is the "structural newline" — it guarantees every line has a terminator and
   means a cursor can always sit on a valid character. Without it, the very
   last position in the file would be undefined, and every line-iteration
   algorithm would need a special case.

2. **Non-empty**: at least one character must exist. This follows from the
   trailing newline but is worth naming explicitly because several algorithms
   assume it.

Every cursor position must also satisfy:

3. **In-bounds**: both ends of a selection must fall within the buffer's
   length. A selection pointing past the end of the buffer is nonsense.

## When an invariant is about to break: three options

Suppose a plugin submits a changeset that would delete the trailing newline.
Three responses are possible:

**Option 1 — silent repair.** Detect the violation and append the missing
newline automatically before the caller sees the result.

This feels safe but is treacherous. A changeset carries a "resulting length"
that the rest of the edit algebra depends on. If we silently append a character
to fix the buffer, the resulting length is now off by one. Subsequent
operations — composing two changesets, inverting an edit for undo — all
silently use the wrong length. The bug doesn't surface where the repair
happened; it surfaces as a wrong cursor position or a length mismatch later,
in completely unrelated code. Silent repairs make bugs invisible at the source
and visible far away.

**Option 2 — crash immediately.** Detect the violation and panic.

Loud failure is better than invisible corruption — at least the source is
obvious. But crashing the editor because one plugin made a mistake is too
drastic. A broken plugin should not take down every open buffer.

**Option 3 — reject the operation and return an error.** Leave the buffer
unchanged; hand the error back to the caller to decide what to do.

This is the right choice at the *trust boundary* — the point where code from
outside the editor core enters the system. The caller (either the editor layer
or a plugin) can log the error, skip the bad command, and keep running. The
buffer is untouched.

## Where to check

There are two kinds of call sites:

- **Internal commands** (`insert-char`, `delete-char`, motion code): these
  build changesets by construction and can never violate the invariants. They
  expect success and treat a failure as an engine bug — a hard crash with a
  diagnostic message is appropriate here.

- **The plugin entry point**: a plugin assembles a changeset from scratch and
  submits it. This is the only place where untrusted data enters the system.
  This single entry point validates the operation and returns an error on
  failure.

Adding validation to every internal function would be noise — forcing internal
code to handle errors that provably cannot occur. The right design is:
**validate once at the boundary, trust everything inside**. The boundary is
narrow and well-defined.

During development, internal code uses lightweight assertions that only run in
debug builds. These assertions catch engine bugs during testing without paying
any cost in release builds.

## Reverting on failure without explicit cleanup

Consider the sequence: build an inverse changeset (for undo), apply the
forward changeset, check the invariants. If the invariant check fails, the
forward changeset is rejected — but we already built the inverse. Do we need
to clean it up?

No. The inverse is just a value on the stack. When the failure branch is taken
and execution leaves that scope, the value is automatically freed. There is
nothing to clean up.

This is a small example of how Rust's ownership model turns resource management
into a mechanical property of the language rather than a manual obligation.
Allocating a temporary, using it on the success path, and automatically
discarding it on the failure path requires zero explicit cleanup code.

```rust
let inv_cs = cs.invert(&buf);       // build the inverse while buf is still original
match cs.apply(&buf) {
    Ok(new_buf) => { /* push inv_cs to undo stack */ }
    Err(e)      => { /* inv_cs is automatically freed here — no cleanup needed */ }
}
```

## Why the apply function takes the buffer by reference

The original version consumed the buffer (taking it by value). That was an
intentional optimization: the buffer's underlying rope could be mutated in
place rather than cloned.

The problem: if `apply` failed, the buffer was gone. The caller had no way to
recover the original.

The fix: take the buffer by reference instead. Cloning the rope before mutating
costs almost nothing because the rope's tree is built from reference-counted
nodes — cloning just bumps a reference count. Apply then works on the clone,
checks the post-conditions, and only wraps the clone in a new buffer if
everything succeeded. On failure, the clone is dropped and the original is
intact.

The key insight is that "recoverable failure" and "mutation in place" are
in tension. The optimization that worked when the operation always succeeded
becomes a liability when it can fail. Taking a reference and cloning a
reference-counted tree is the pragmatic middle ground.

## A note on failure modes

Two distinct failure modes can occur:

- **Changeset failure**: the changeset's declared "length before" doesn't
  match the actual buffer, or applying it would destroy the trailing newline.
- **Selection failure**: after the changeset applies successfully, one of the
  cursor positions falls out of bounds.

These are separate errors. The plugin entry point checks both in sequence:
apply the changeset first, then validate the selection against the resulting
buffer. If either step fails, the entire operation is rejected and the original
buffer is returned.

Separating the two keeps each failure type narrow and diagnosable — a plugin
author seeing "selection out of bounds" knows their cursor math is wrong, not
their changeset.
