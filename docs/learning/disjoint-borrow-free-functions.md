# Disjoint Borrows and Free Functions

## The problem in plain terms

Rust enforces a rule about shared state: at any given moment, you can have
many read-only views into a value *or* one exclusive write access — never both
at once. This rule is checked at compile time, not at runtime.

For simple cases the compiler is smart enough to see that two fields on the
same struct are independent. Reading `editor.kill_ring` while writing
`editor.buffers` is fine — they don't overlap.

The problem arises when you wrap operations in methods on a struct. A method
that takes `&mut self` (exclusive write access) borrows the *entire* struct,
not just the fields it touches. So if you call a `self.some_method()` that
touches `buffers`, and you also hold a reference to `self.kill_ring`
somewhere in the same scope, the compiler rejects it — even though the two
fields are completely separate.

The workaround the codebase used to reach for was defensive copying: clone the
value you need before calling the method, so the borrow ends. Reasonable for
one place, but HUME's editor state struct has roughly 35 fields and command
bodies cross into it constantly — those clones added up to about 52 per common
keystroke.

## The rule

> **Inside a method that takes exclusive write access to `self`, never call
> another method that also takes exclusive write access to `self`.**
> Read and write fields directly. Delegate logic to standalone functions that
> take only the specific fields they need.

When you pass `editor.kill_ring` and `editor.buffers` as separate arguments to
a standalone function, the compiler can see that they're separate. The proof
doesn't go through a method boundary — it goes through the names of individual
fields.

## Before and after

Here is the same operation written both ways.

**The old way — a method on the editor struct:**

```rust
// Inside some &mut self method that needs kill_ring:
let values = self.kill_ring.head().to_vec();  // clone to end the borrow
self.doc_edit_grouped(|b, s| paste(b, s, &values));  // now compiles, but one allocation wasted
```

`to_vec()` creates a copy so the borrow of `kill_ring` ends before
`doc_edit_grouped` — which also needs `&mut self` — is called.

**The new way — a standalone function:**

```rust
// kill_ring and buffers are different fields → the compiler can prove they don't overlap:
let values = self.kill_ring.head();  // no clone
doc_ops::apply_doc_edit_grouped(
    &mut self.buffers, &mut self.pane_state, pid, bid,
    |b, s| paste(b, s, values)
);
```

No allocation. The compiler is satisfied because `kill_ring`, `buffers`, and
`pane_state` are provably separate things.

## A related trick: move-and-put-back

Some commands need to transform a value that lives inside a struct, but they
can't borrow it while they're also writing to a sibling field. The alternative
to cloning is to *move the value out*, transform it, and put it back:

```rust
// Clone version — allocates a full copy of the selection set:
let sels = pane_state.selections.clone();
pane_state.selections = transform(sels);

// Move-and-put-back — zero allocation:
let sels = std::mem::take(&mut pane_state.selections);  // leaves a minimal placeholder
pane_state.selections = transform(sels);
```

`mem::take` moves the value out of the struct and leaves a defined
(but minimal) placeholder behind. The struct is temporarily in a valid but
empty state during the transform — there is no window where it is
inconsistent, because the code between `take` and the write-back is
synchronous.

## Direct disjoint field access

When two fields are genuinely separate, you can hold a read reference to one
while writing the other — without any method call getting in the way:

```rust
// pane_transient and pane_state are separate fields.
// The compiler can prove they don't overlap when accessed directly:
if let Some(sels) = self.pane_transient[pid].pre_search_sels.as_ref() {
    self.pane_state[pid][bid].selections = sels.clone();
    //   ^^^^^^^^^^^^^^^^^^^^^^^^       ↑ write
    //   ^ shared read from another field — both allowed simultaneously
}
```

Previously this required a separate `set_current_selections(&mut self, …)`
method, which locked the entire struct and forced a clone. Direct field access
eliminates both problems at once.

## Entry points stay as methods

The editor's top-level entry points — `handle_key`, `execute_keymap_command` —
still take exclusive write access to the whole struct. That's fine; they are
*thin delegators*: they extract the specific fields they need and pass them
to standalone functions immediately, rather than calling other methods on self.

```rust
fn begin_edit_group_current(&mut self) {
    let pane_id = self.focused_pane_id;
    let buf_id  = self.focused_buffer_id();
    doc_ops::begin_edit_group(&self.buffers, &mut self.pane_state, pane_id, buf_id);
    //                         ^^^^^^^^^^^^   ^^^^^^^^^^^^^^^^^^
    //                         two separate fields passed as separate arguments
}
```

## Why this matters

The benefit isn't just the removed allocations (though 52 allocations per
keystroke is not trivial). The deeper benefit is that the rule is *mechanical*
and can be enforced automatically. HUME has a compile-time lint that scans
command and dispatch code and fails the build if any method on the deny-list
is called from inside another method. The code organisation that produced the
allocations was the same code organisation that made the borrow checker
unhappy — fixing the architecture fixed both.

> **For Rust newcomers:** the concept behind this section is called
> "Non-Lexical Lifetimes" (NLL). NLL is the compiler's ability to track that
> a borrow ends at the last point where it's used, not at the end of the
> enclosing scope. NLL can prove fields are disjoint when accessed directly,
> but it cannot see through method boundaries — a method that takes `&mut self`
> appears to borrow everything, even if it only touches one field. This is a
> known limitation; standalone functions are the idiomatic workaround.
