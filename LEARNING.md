# HUME — Learning Notes

Concepts that come up while building HUME, explained in enough depth to be
useful later. Sections are added as topics arise during development.

---

## Unicode Position Model: Bytes, Chars, and Grapheme Clusters

Understanding this hierarchy is essential for HUME's architecture. Three
different units can describe a "position" in text, and choosing the wrong one
at the wrong layer causes subtle, hard-to-reproduce bugs.

### Byte offset

A byte offset is a raw index into memory. In UTF-8 (Rust's string encoding),
characters are **variable-width**: 1 to 4 bytes each.

```
"café"
 c  a  f  é
 1  1  1  2   ← bytes per character
```

| Char | Bytes   | Byte offsets |
|------|---------|-------------|
| `c`  | `63`    | 0 |
| `a`  | `61`    | 1 |
| `f`  | `66`    | 2 |
| `é`  | `C3 A9` | 3, 4 |

`é` occupies bytes 3 **and** 4. Byte offset 4 points into the **middle** of a
character — it is not a valid character boundary. This is why `s[3..4]` on
`"café"` panics in Rust: slicing through a multi-byte character is undefined.

Byte offsets are used internally by Rust's `str` and by `ropey`, but they are
**never exposed across module boundaries** in HUME. They are an implementation
detail.

### Char offset

A char offset counts **Unicode scalar values** (Rust's `char` type),
regardless of how many bytes each one takes.

```
"café"
 c  a  f  é
 0  1  2  3   ← char offsets
```

`é` is a single `char` at offset 3 — no partial-character hazard. This is
`ropey`'s native addressing unit, and it is what HUME's `Buffer`, `Selection`,
and `SelectionSet` use for all positions.

Char offsets make sense for an editor at the storage layer:
- `insert(at, text)` and `remove(from, to)` can be expressed cleanly.
- `anchor` and `head` in a `Selection` are meaningful without knowing the
  encoding of any particular character.

### Grapheme cluster

A char offset solves the byte problem, but there is a level above it:
**grapheme clusters** — what a user perceives as a single indivisible
character, which may be composed of multiple Unicode scalar values.

```
"é"  can be:
  U+00E9             → 1 char  (precomposed NFC form)
  U+0065 + U+0301    → 2 chars (base 'e' + combining acute accent)

"👨‍👩‍👧"              → 1 visible character, but 5 chars
                       (joined with zero-width joiners U+200D)
```

Pressing the right-arrow key on `"👨‍👩‍👧"` should advance the cursor past the
entire emoji in one step, not stop five times. This is the job of
`grapheme.rs`: it takes a `RopeSlice` and returns the next/previous **valid
grapheme boundary** as a char offset.

### Architectural rule

| Unit | Granularity | Role in HUME |
|------|-------------|--------------|
| Byte offset | Raw memory | Internal to `ropey` — never exposed |
| Char offset | Unicode scalar value (`char`) | Storage, selections, `Buffer` API |
| Grapheme cluster | User-perceived character | Cursor movement, motions (`grapheme.rs`) |

The boundary between layers is strict: `grapheme.rs` **consumes** char offsets
and **produces** char offsets that happen to land on grapheme boundaries.
Everything above it works purely in char offsets and never needs to know about
bytes or grapheme internals.

---

## Edit Operations: Acting on Selections

### The select-then-act model

In HUME, edit operations never act on a bare cursor position. They act on a
`SelectionSet` — which is always a `Vec<Selection>`. Selections are always
**inclusive**: `anchor == head` is a 1-char selection covering the character at
that index, not a zero-width point. Each `Selection` is either:

- **Single-character** (`anchor == head`): the cursor sits on exactly one character.
- **Multi-character** (`anchor != head`): a contiguous region of selected text.

An operation like "insert character `x`" means:

- For a **single-character selection**: insert `x` before the cursor character;
  the cursor advances to the next character.
- For a **multi-character selection**: replace the entire selected region with `x`.

This is the same rule in both cases. Single-cursor editing, visual-mode editing,
and multicursor editing all fall out of the same loop.

### Multi-selection edit ordering

A `SelectionSet` can contain multiple selections simultaneously (multicursor).
When an edit touches multiple positions, **the order of application matters**:
inserting a character at offset 0 shifts every position to its right, so
naively applying edits one-by-one would corrupt subsequent offsets.

HUME avoids this entirely with `ChangeSetBuilder`: all input positions are
expressed in **original-buffer coordinates**, and the builder handles the
translation internally. See the Changesets section below.

---

## Changesets: Describing Edits as Data

### What is a changeset?

A changeset is a **compact, invertible description** of a document
transformation. Instead of mutating the buffer for each selection, we build
one changeset that describes all the edits, then apply it once.

The representation is a sequence of three operations:

| Operation | Meaning |
|-----------|---------|
| `Retain(n)` | Skip `n` chars unchanged |
| `Delete(n)` | Remove `n` chars from the old doc |
| `Insert(s)` | Add `s` to the new doc |

**Example:** Insert `!` at positions 0 and 6 in `"hello world"`:

```
Insert("!"), Retain(6), Insert("!"), Retain(5)
```

This single object describes the entire multi-cursor edit. `apply` consumes
the buffer and executes Delete/Insert operations directly on the rope —
each O(log n). Retain operations are free (the chars are already there).
Total cost: O(k log n) for k non-retain operations.

Because `apply` consumes the buffer, the old buffer no longer exists after
application. This is fine — the inverse changeset captures everything needed
to reconstruct it (see undo/redo below). The caller must call `invert`
before `apply` if it needs the inverse.

### Why not just mutate the buffer directly?

Direct mutation (clone + edit per selection) works, but the edit is lost
after application — there is no record of what changed. A changeset preserves
the edit as data, which enables:

1. **Undo/redo.** Invert the changeset to get an undo operation:
   - `Retain(n)` → `Retain(n)` (no change)
   - `Delete(n)` → `Insert(deleted text)` (re-insert what was removed)
   - `Insert(s)` → `Delete(len(s))` (remove what was added)

   Applying the inverse to the result buffer gives back the original.

2. **Composition.** Two sequential changesets A→B and B→C can be merged into
   a single A→C changeset. This is essential for grouping keystrokes into
   undo steps (typing a word should undo as one operation, not per-character).

3. **Position mapping.** Given a position in the old document, `map_pos`
   computes where it ends up in the new document — accounting for all
   insertions and deletions. An `Assoc` parameter (`Before`/`After`) controls
   which side of an insertion the position sticks to.

   Note that edit operations and undo/redo never call `map_pos`. Edit
   operations use `new_pos()` directly; undo/redo restores selections from the
   stored `Transaction` (see below). `Assoc` is reserved for **external
   positions** — things that exist independently of any specific edit, like LSP
   diagnostic ranges or bookmarks. When a diagnostic sits at offset 5 and text
   is inserted at offset 5, `Assoc::Before` keeps it glued to the left of the
   insertion; `Assoc::After` pushes it past.

### The builder pattern

Edit operations build changesets incrementally using `ChangeSetBuilder`. The
builder tracks two cursors:

- `old_pos` — how far we have consumed in the old document
- `new_pos` — how far we have produced in the new document

This dual tracking replaces the manual delta accumulator. After each
`insert()` call, `new_pos()` tells you exactly where a cursor should land
in the result buffer.

```text
Builder state for insert_char('x') with cursor at offset 3 in "hello":

  b.retain(3)     →  old_pos=3, new_pos=3    (skip "hel")
  b.insert("x")   →  old_pos=3, new_pos=4    (insert 'x')
  b.retain_rest()  →  old_pos=5, new_pos=6    (keep "lo")

  Result: Retain(3), Insert("x"), Retain(2)
  Cursor: b.new_pos() at time of insert = 4  →  "helx|lo"
```

All positions are in **original-buffer space** — no delta tracking, no
intermediate buffer clones. The builder handles the coordinate translation
internally.

### Transactions: changesets with cursor state

A `ChangeSet` describes only the text change. A `Transaction` pairs it with the
**post-apply** `SelectionSet` — where the cursors land *after* the changeset is
applied. This invariant holds for every Transaction, forward or inverse.

At edit time you build **two** Transactions from the same changeset:

```text
// 1. Capture the inverse BEFORE apply consumes the buffer.
let inv_cs  = cs.invert(&old_buf);
let new_buf = cs.apply(old_buf);          // old_buf is consumed here

// 2. Build both Transactions from the same cs/inv_cs.
let forward = Transaction::new(cs,     post_edit_sels);  // for redo
let inverse = Transaction::new(inv_cs, pre_edit_sels);   // push to undo stack
```

The inverse Transaction's `selection` is `pre_edit_sels` — the cursors from
*before* the edit — because that is where applying the inverse will leave the
cursors. The "always post-apply" invariant holds: after running the inverse,
the cursors are at `pre_edit_sels`.

**Timing matters.** `invert(&old_buf)` must be called before `apply(old_buf)`.
`apply` consumes the buffer (the old rope is gone). `invert` needs the original
content to reconstruct the `Insert` operations for deleted text — it captures
the deleted chars from the live rope at inversion time.

The history manager stores inverse Transactions. Applying one restores both the
text and the cursor positions in a single step.

### Implementation

- `src/changeset.rs` — `Operation`, `ChangeSet`, `ChangeSetBuilder`, `Assoc`
- `src/transaction.rs` — `Transaction` (pairs ChangeSet with SelectionSet)
- `src/edit.rs` — edit operations build changesets via the builder

---

## Buffer Invariants and Plugin Safety

### The invariants

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

### Fail fast, don't silently repair

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

### Validate at the trust boundary, not everywhere

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

### `apply` takes `&Buffer`, not `Buffer`

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
    if rope.char(rope.len_chars() - 1) != '\n' { return Err(...); }
    Ok(Buffer::from_rope(rope))
}
```

### Inverse changeset cleanup is free

When `apply` fails, the caller may have already built an inverse changeset
(for the undo stack). There is no need for explicit cleanup: the inverse is
just a `ChangeSet` on the stack. Rust's ownership model drops it automatically
when the error branch is taken and it goes out of scope.

```rust
let inv_cs = cs.invert(&buf);          // build inverse first
match cs.apply(&buf) {
    Ok(new_buf)  => { /* push inv_cs to undo stack */ }
    Err(e)       => { /* inv_cs dropped here — no cleanup needed */ }
}
```

This is a good example of ownership making resource management "fall out for
free". In a garbage-collected language you would need to either let the GC
collect it eventually or explicitly null the reference; in Rust it is
deterministic and requires no code.

### Error type design

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

### `debug_assert` vs `assert` vs `Result`

| Mechanism | Fires in | Use when |
|---|---|---|
| `debug_assert!` | debug + test builds only | Internal invariant that trusted code should never violate; catching it in tests is enough |
| `assert!` / `assert_eq!` | all builds (panics) | Structural precondition that is unrecoverable and implies a programming error (e.g. `ChangeSetBuilder::finish` verifies you consumed all chars) |
| `Result::Err` | all builds (recoverable) | Trust boundary — caller provided invalid data that the system can reject gracefully |

HUME's rule: `debug_assert` for engine internals, `assert` for builder
contracts that imply a programming mistake too severe to recover from, `Result`
for anything that crosses the plugin boundary.
