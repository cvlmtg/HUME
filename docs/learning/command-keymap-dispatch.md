# The Command/Keymap/Dispatch Architecture

HUME's key handling is split across four layers, each owning one
responsibility. Understanding the split — and what each layer does *not* know —
is the key to extending the editor safely.

## The four layers

| Layer | Role | Knows keys? | Knows editor state? |
|-------|------|-------------|---------------------|
| **Registry** | Name → command lookup | No | No |
| **Commands** | What each command does | No | Yes |
| **Keymap** | Key sequence → command name | Yes | No |
| **Dispatch** | Resolve name, call command | No | Yes |

The keymap layer knows about keys but not about what any command does. The
command layer knows what each command does but not which key triggered it. The
registry bridges them by name. The dispatch layer glues everything together.

## Layer 1: The Registry

Every user-facing operation is a named command — a name string paired with a
function and some metadata. The registry is the single source of truth for what
commands exist.

Commands come in a few varieties:

- **Motion** commands move the cursor by returning a new position. They are
  pure: they take the current buffer and cursor state and return new cursor
  state, with no side effects.
- **Selection** commands similarly return a new selection state without
  modifying the buffer.
- **Edit** commands return a new buffer *and* new cursor state, plus a
  changeset (the description of what changed, used for undo).
- **Editor-level** commands have access to the full editor state: mode
  changes, registers, undo groups. Used for operations that don't fit the pure
  model.
- **Script-backed** commands are defined in Steel (HUME's scripting language)
  and stored by name. The command table maps the command name to its handler
  separately; the command itself carries its name plus metadata, never a
  pointer to the procedure. Both lookups happen by name at dispatch time.
- **Lazy** commands are placeholder stubs registered up front for commands a
  plugin has declared but not yet loaded. The first dispatch of one triggers
  the owning plugin's body, which replaces the stub with the real
  implementation.

The first three variants are pure functions — they cannot touch anything
outside their inputs. This makes them trivially testable and composable.

### Extend mode is a runtime parameter, not separate commands

There is no `"extend-left"` or `"extend-select-word"` command. Each base
command (e.g. `"move-left"`, `"select-word"`) receives a move/extend
parameter at dispatch time when the user is in extend mode. The command
branches internally:

- **Move** mode: the anchor collapses to the new cursor position (a fresh
  one-character selection).
- **Extend** mode: the anchor stays fixed, only the head moves — so the same
  parameter produces a grow or a shrink depending on which way the head moves
  relative to the anchor, with no separate command needed for either.

This means adding a new motion requires **one function and one registration**
— the extend variant comes for free from the parameter.

A command may also register an *alternate body* chosen at dispatch time by a
buffer option. Word motions ship this way: one command name, with the
whitespace-including variant swapped in per buffer while
`word-selects-whitespace` is on.

### The extendable flag

Commands declare at registration time whether they *have* extend semantics.
The flag is a guard, not a trigger: the Ctrl+key one-shot mechanism
(described below) only fires for commands that do — a Ctrl+key resolving to
a command without extend semantics (like undo) is suppressed as a no-op
rather than run. Bindings that should *always* extend are a separate,
per-binding declaration (see "Explicit force-extend bindings" below).

All Steel-defined commands are extendable automatically. When Ctrl+key delivers
extend to your command, the lambda receives `extend = #t` as its second argument
(if the lambda declares a second parameter). The body can then forward it:

```scheme
(define-command! "step-right" "Move right N times, optionally extending."
  (lambda (count extend)
    (call! "move-right" count extend)))
```

Steel commands can also opt in to dot-repeat (`.`) via `#:repeatable #t`. By
default they are non-repeatable — only the author knows whether re-running the
full command body at a new cursor position is meaningful. Shell-out commands
(`#:inline-output #t`) cannot be repeatable; passing both flags is an error.

```scheme
(define-command! "delete-selection" "Delete current selection; repeatable."
  (lambda () (call! "delete"))
  #:repeatable #t)
```

### Typed commands (`:` commands)

The registry also holds commands invoked from the `:` command line. They share
the same name namespace as key-bound commands, which prevents collisions and
gives a single source for future `:help` and command-palette features.

## Layer 2: Commands

The command implementations are plain functions: they take the buffer, the
current selections, and a count, and return new state. No knowledge of keys,
modes, or how they were invoked.

## Layer 3: The Keymap

The keymap is a trie — a tree structure where each node represents one key in a
sequence — that maps key sequences to command names. The stored value at each
leaf is just a name string, not a function pointer.

This separation is deliberate: the Steel scripting layer rewrites keymap entries
to support user-defined keymaps. A user remap is just `key → command-name`.
The trie can be rewritten without touching any execution logic.

Three types of trie nodes:

- **Leaf**: a complete binding → dispatch the named command.
- **Interior**: more keys needed (e.g. `m` → `i` → `w` for inner-word).
- **WaitChar**: the next keypress is consumed as a character argument
  (used by find, replace, etc.).

### Three keymaps

HUME maintains three separate tries:

| Trie | Purpose |
|------|---------|
| Normal | Main keymap for Normal mode |
| Extend | Sparse overrides for Extend mode (checked first) |
| Insert | Single-key bindings for Insert mode |

The **extend trie** ships empty by default — flipping anchor and head (Vim's
visual `o`) is already reachable via `Ctrl+e` in both Normal and
Extend mode, so no override is needed out of the box. Any key not found in the
extend trie falls through to the normal trie with extend mode active, which
applies extend semantics automatically.

This lets Steel customise per-key extend-mode overrides: "when in extend mode
and the user presses this key, run this specific command instead of the usual
one." A keybinding plugin that prefers Vim's `o` over `Ctrl+e`, for example,
can bind `o` in the extend trie to the same flip command.

## Layer 4: Dispatch

The dispatcher is the glue. It receives a command name and an extend flag,
converts those to the appropriate mode parameter, looks up the command in the
registry, and calls it.

The flow on any keypress:

```
keypress
  → keymap trie walk     → a command name (e.g. "move-right")
  → dispatcher           → converts extend flag to move/extend mode
  → registry lookup      → the function for "move-right"
  → call                 → function(buffer, selections, count, mode)
```

### Three ways to get Extend mode

**1. Sticky extend mode.** The user presses `e` to enter Extend mode. All
subsequent commands run with Extend semantics until the mode is exited. The
extend trie is checked first for per-key overrides. Acting destructively on
the selection (delete, paste, replace) also exits Extend mode automatically —
mirroring Vim's visual-mode operators; yank and pure motions leave it active.

**2. Ctrl+key one-shot extend (kitty keyboard protocol).** When kitty protocol
is active, pressing `Ctrl+l` strips the Control modifier, looks up `l` in the
normal trie (`"move-right"`), and dispatches with Extend mode. Works only on
kitty-capable terminals; silently absent on legacy terminals.

**3. Explicit force-extend bindings.** Some keybindings are declared to always
extend — for example, `Ctrl+x` always accumulates line selections, even without
sticky extend mode or kitty. This works on any terminal.

To remap a command with its extend behaviour to a different key:

```scheme
(bind-key! 'normal "f" "select-line")          ; Move mode
(bind-key-extend! 'normal "C-f" "select-line") ; always extends (force-extend)
```

The user only writes the base command name — there is no extend-variant name
to learn. Note that one-shot extend is automatic only for an *unbound*
Ctrl+letter (on kitty terminals); binding the key explicitly takes over, so
a binding that should always extend must say so via `bind-key-extend!`.

### Counts: distinguishing a bare keypress from an explicit count

Typing `3w` should behave differently from typing `w` three times in a row in
one respect: dot-repeat and a few other bookkeeping paths care whether a count
was actually typed, not just what number ends up being used. So the dispatcher
tracks count as "no count was typed" versus "an explicit count of *n* was
typed" — a bare `w` and an explicit `1w` both move by one word, but they are
distinguishable to the layers above dispatch.

Script-defined commands receive count and extend as their first two
parameters (if declared), the same injection mechanism as the extend flag
described above. Since Scheme has no built-in way to say "this argument was
omitted," dispatch passes a count of zero to mean "no count was typed," and a
command that forwards its count to another command decodes zero back into
"no count" before passing it on — so a bare keypress stays a bare keypress
all the way through a chain of commands calling each other.

### WaitChar: parameterized commands

Some commands need a character argument: find-forward, find-backward, replace.
The keymap stores these as *WaitChar* nodes.

When the trie walk hits a WaitChar node, the dispatcher saves the command name
and waits. The next keypress is consumed as the character argument, and then
the command runs with that character. Extend mode is resolved at the moment the
character arrives — not at the moment the trigger key is pressed.

## Commands are mode-agnostic

Commands in the registry have no mode affinity. If Steel binds `"flip-selections"`
to a key in the insert keymap, and the insert handler resolves a leaf, it calls
the same dispatch path — the selection flips, the editor stays in Insert mode.
Whether that binding is useful is the user's responsibility. The editor doesn't
second-guess it.

## Insert mode limitations

Insert mode's keymap walk is single-key only. Multi-key sequences and WaitChar
commands won't work there — insert mode is optimised for typing, not for
command choreography. Single-key leaf commands bind fine.

## Independence of layers

The value of the split is that any layer can change without touching the others:

- **New command**: add the function, register it, bind a key. The dispatch
  layer is unchanged.
- **Rebind a key**: only touch the keymap.
- **Change dispatch** (e.g. add macro recording): only touch the dispatcher.
- **User keymaps via Steel**: rewrite keymap trie entries. The registry and
  dispatch layer are unaffected.

A change in one layer cannot corrupt another because they communicate only
through name strings.

A related invariant is enforced at the dispatch layer: every native command's
function body must run through one funnel. A test in the suite scans the
editor's source for any second place that calls a native command's function
directly, and fails if it finds one. The funnel is where the bookkeeping that
surrounds every command — dot-repeat, paste-session commits, jump-list
updates, extend-mode auto-exit — gets applied. Letting a
second call site bypass it would mean two paths for the same command, and the
bookkeeping would silently regress on whichever path skipped the funnel;
tests that pin the primary effect would stay green either way. The lint makes
that mistake a test failure rather than a behavioural drift.
