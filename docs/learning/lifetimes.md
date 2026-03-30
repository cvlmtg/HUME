# Lifetimes: Borrowing Across Struct Boundaries

## The problem lifetimes solve

Rust's borrow checker guarantees that a reference never outlives the value it
points to. This is easy to enforce inside a single function — the compiler can
see both the value and the reference, and check that the reference is dropped
first.

The problem arises when you put a reference **inside a struct**:

```rust
struct DisplayLine {
    content: &RopeSlice,   // ← which RopeSlice? how long does this live?
}
```

The compiler has no way to know when `content` will be used, so it cannot
verify safety. The lifetime annotation `&'a` is the solution: it gives the
compiler a name to reason about.

## What `'a` means

`'a` is a **lifetime parameter** — a label that says "these references all
point into the same scope and will not outlive it." The `'a` is not a duration
or a timer; it is a constraint.

```rust
pub(crate) struct DisplayLine<'a> {
    pub content: RopeSlice<'a>,
    pub line_number: Option<usize>,
    pub char_offset: Option<usize>,
}
```

This declaration says: "`DisplayLine` borrows from some scope. All references
tagged `'a` must remain valid for at least as long as this `DisplayLine`
exists." When the `DisplayLine` is dropped, those borrows are released — and
the compiler verifies this statically.

The `'a` on the struct and the `'a` on each field are the **same label**. The
compiler unifies them: every `&'a T` field must be borrowed from a scope that
outlives the struct instance.

## How it looks at the call site

In `renderer.rs`, display lines are computed from the buffer for one frame:

```rust
let display_lines = view.display_lines(buf);
// display_lines borrows from buf — 'a is the lifetime of buf
for dl in &display_lines {
    render_gutter(screen_buf, editor, dl, x, y);
    // dl: &DisplayLine<'_> — borrow of buf is live here
}
// display_lines dropped here — borrow of buf ends
```

`buf` outlives the loop, so the compiler accepts this. If you tried to store
`display_lines` in a field of `Editor`, the compiler would reject it — the
borrow of `buf` would outlive the frame.

## Lifetime elision: when you don't see `'a`

Rust can infer lifetimes in simple cases and lets you omit them. Function
signatures are the main beneficiary:

```rust
// Written explicitly:
fn render_gutter<'a>(editor: &'a Editor, dl: &DisplayLine<'a>, ...) { ... }

// What you actually write (elision rules fill in the 'a):
fn render_gutter(editor: &Editor, dl: &DisplayLine<'_>, ...) { ... }
```

The `'_` is an anonymous lifetime — "some lifetime, inferred by the compiler."
It signals that a lifetime exists without naming it. You will see `'_` often
in HUME where the lifetime does not need to be shared across multiple
parameters.

## The rule of thumb

- **Named `'a`**: two or more places in the same signature need to share the
  same lifetime (e.g., a struct that holds multiple borrows from the same
  source).
- **`'_`**: one reference, one place, the compiler can figure it out.
- **No annotation**: a function that takes no references, or only owned values.
