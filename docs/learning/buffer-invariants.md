# Buffer Invariants and Plugin Safety

## The invariants

Every `Buffer` must satisfy two invariants at all times:

1. **Trailing newline**: the rope always ends with `\n`. This is the
   "structural newline" — it guarantees every line has a terminator, and it
   means cursors can always sit on a valid character (the last char is always
   accessible, never past-the-end).

2. **Non-empty**: `len_chars() >= 1`. Follows from the trailing newline, but
   worth naming explicitly because several algorithms assume it.

Every `Selection` must also satisfy:

3. **In-bounds**: `head < buf_len` and `anchor < buf_len` for the buffer it is
   paired with.

## Fail fast, don't silently repair

When an invariant is about to be violated, there are three options:

1. **Silent repair** — e.g. append the missing `\n` automatically.
2. **Panic** — crash immediately with a message.
3. **Return `Err`** — reject the operation, leave the original state untouched.

Silent repair is tempting but dangerous. A changeset carries metadata
(`len_after`) that the rest of the algebra relies on. A repair that appends
`\n` changes the buffer length but not `len_after`, so `compose`,
`invert`, and `map_pos` all silently operate on the wrong value. The corruption
is invisible until it manifests as a wrong cursor position or a length-mismatch
panic somewhere else entirely.

Panicking is better — it fails loudly at the source — but it crashes the editor
for a mistake in one plugin. An editor should be resilient: a broken plugin
should not take down other buffers.

**Return `Err`** is the right choice at the trust boundary, for the same reason
`Result` is preferred over `panic` in Rust library code: the caller can decide
how to handle the failure. In HUME, `Transaction::apply` is that boundary. A
plugin that assembles an invalid `Transaction` gets a clear error; the original
buffer is unmodified; the editor continues running.

## Validate at the trust boundary, not everywhere

There are two kinds of call sites:

- **Internal commands** (`insert_char`, `delete_char_forward`, etc.): these
  build changesets by construction and are always correct. They call
  `ChangeSet::apply` directly and use `.expect()` — a panic here is a bug in
  the engine, not a plugin mistake.

- **Plugin entry point** (`Transaction::apply`): a plugin assembles a
  `Transaction` from raw parts. This is the one place where untrusted data
  enters the system. `apply` validates here and returns `Result`.

Adding `Result` to every internal function would be noise — it would force
callers to handle errors that can never happen. The right design is: validate
once at the boundary, trust everything inside.

`debug_assert!` fills the gap for internal code: these assertions run during
development and tests (where you want loud feedback on engine bugs) but compile
to nothing in release builds (where the cost would be wasted).

## `apply` takes `&Buffer`, not `Buffer`

The original `ChangeSet::apply` consumed the buffer (`buf: Buffer`). This was
an optimization: the rope is mutated in-place rather than cloned. But consuming
the buffer creates a recovery problem — if `apply` fails, the original buffer
is gone.

The solution is to take `&Buffer` instead. Ropey's `Rope::clone()` is **O(1)**
because the tree is built from `Arc`-counted nodes: cloning a rope just bumps a
reference count. `apply` clones the rope, mutates the clone, checks the
post-conditions, and only on success wraps the clone in a new `Buffer`. If
anything goes wrong, the clone is dropped and the caller still holds the
original.

```rust
pub(crate) fn apply(&self, buf: &Buffer) -> Result<Buffer, ApplyError> {
    if buf.len_chars() != self.len_before { return Err(...); }
    let mut rope = buf.rope().clone();   // O(1)
    // ... mutate rope ...
    if rope.len_chars() == 0 || rope.char(rope.len_chars() - 1) != '\n' { return Err(...); }
    Ok(Buffer::from_rope(rope, buf.line_ending()))
}
```

## Inverse changeset cleanup is free

When `apply` fails, the caller may have already built an inverse changeset
(for the undo stack). There is no need for explicit cleanup: the inverse is
just a `ChangeSet` on the stack. Rust's ownership model drops it automatically
when the error branch is taken and it goes out of scope.

```rust
let inv_cs = cs.invert(&buf);           // build inverse first
match cs.apply(&buf) {                  // apply takes &buf — original intact
    Ok(new_buf)  => { /* push inv_cs to undo stack */ }
    Err(e)       => { /* inv_cs dropped here — no cleanup needed */ }
}
```

This is a good example of ownership making resource management "fall out for
free". In a garbage-collected language you would need to either let the GC
collect it eventually or explicitly null the reference; in Rust it is
deterministic and requires no code.

## Error type design

`TransactionError` wraps two distinct failure sources:

- `ApplyError` — changeset-level failures (`LengthMismatch`, `TrailingNewlineMissing`)
- `ValidationError` — selection-level failures (`SelectionOutOfBounds`, `EmptyBuffer`)

The `From` trait lets `?` convert each into `TransactionError` automatically:

```rust
pub(crate) fn apply(&self, buf: &Buffer) -> Result<(Buffer, SelectionSet), TransactionError> {
    let new_buf = self.changes.apply(buf)?;        // ApplyError → TransactionError via From
    self.selection.validate(new_buf.len_chars())?; // ValidationError → TransactionError via From
    Ok((new_buf, self.selection.clone()))
}
```

This is the standard Rust pattern for layering error types: inner functions
return narrow errors; outer functions define a wider type that covers all their
failure modes; `From` + `?` compose them with no boilerplate.

## `debug_assert` vs `assert` vs `Result`

| Mechanism | Fires in | Use when |
|---|---|---|
| `debug_assert!` | debug + test builds only | Internal invariant that trusted code should never violate; catching it in tests is enough |
| `assert!` / `assert_eq!` | all builds (panics) | Structural precondition that is unrecoverable and implies a programming error (e.g. `ChangeSetBuilder::finish` verifies you consumed all chars) |
| `Result::Err` | all builds (recoverable) | Trust boundary — caller provided invalid data that the system can reject gracefully |

HUME's rule: `debug_assert` for engine internals, `assert` for builder
contracts that imply a programming mistake too severe to recover from, `Result`
for anything that crosses the plugin boundary.
