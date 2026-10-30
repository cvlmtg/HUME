# The Command/Keymap/Dispatch Architecture

Once a key reaches the main editing surface, HUME's key handling is split
across four layers, each owning one responsibility. Understanding the split,
and what each layer does *not* know, is what makes the editor safe to extend.
A key does not always get this far: see "Before dispatch" below.

## The four layers

| Layer | Role | Knows keys? | Knows editor state? |
|-------|------|-------------|---------------------|
| **Registry** | Name → command lookup | No | No |
| **Commands** | What each command does | No | Yes |
| **Keymap** | Key sequence → command name | Yes | No |
| **Dispatch** | Resolve name, call command | No | Yes |

The keymap knows about keys but not about what any command does. A command
knows what it does but not which key triggered it. The registry bridges them
by name, and dispatch glues everything together.

## Layer 1: The Registry

Every user-facing operation is a named command: a name paired with a
function and some metadata. The registry is the single source of truth for
what commands exist.

Commands come in a few varieties:

- **Motion** and **selection** commands take the text, the current
  selections, a count and the move/extend mode, and return new selections.
  They never modify the text.
- **Edit** commands take the text and its selections and return the new text
  with its selections, plus a changeset (the description of what changed,
  used for undo).
- **Editor-level** commands reach the full editor state: mode changes,
  registers, undo groups, and the commands that need a character or a
  pattern from the user (find, replace, search).
- **Script-backed** commands are defined in Steel (HUME's scripting
  language). The registry entry holds the procedure itself, along with its
  name and metadata, so dispatch runs the procedure of the entry it
  resolved.
- **Lazy** commands are placeholder stubs registered up front for commands a
  plugin has declared but not yet loaded. The first dispatch of one runs the
  owning plugin's body, which replaces the stub with the real command.

Motion, selection and edit commands are pure functions that cannot touch
anything outside their inputs, which makes them easy to test and to compose.

### Extend mode is a runtime parameter, not separate commands

There is no `"extend-left"` or `"extend-select-word"` command. Each base
command (`"move-left"`, `"select-word"`) receives a move/extend parameter at
dispatch time, and branches on it:

- **Move** mode: the anchor collapses to the new cursor position (a fresh
  one-character selection).
- **Extend** mode: the anchor stays fixed and only the head moves, so the
  same parameter grows or shrinks the selection depending on which way the
  head moves relative to the anchor.

Adding a new motion therefore takes one function and one registration: the
extend variant comes from the parameter.

The parameter is not the only per-call input a command can read. The word
motions also receive the buffer's `word-selects-whitespace` setting, so one
command body selects the whitespace each buffer asks for.

### The extendable flag

Commands declare at registration time whether they *have* extend semantics.
The flag is a guard, not a trigger. The Ctrl-key one-shot (described below)
only fires for commands that have it: a Ctrl-key resolving to a command
without extend semantics, like undo, is suppressed as a no-op rather than
run. Bindings that should *always* extend are a separate, per-binding
declaration (see "Three ways to get Extend mode" below).

Every Steel-defined command is extendable. Its procedure receives the pane it
was invoked through as its first argument, then the count and the extend
flag, each only if the procedure declares that parameter. The body can
forward them:

```scheme
(define-command! "step-right" "Move right N times, optionally extending."
  (lambda (pane count extend)
    (call! "move-right" pane count extend)))
```

Steel commands can also opt in to dot-repeat (`.`) with `#:repeatable #t`.
They are non-repeatable by default, because only the author knows whether
re-running the whole body at a new cursor makes sense. Commands that hand
the terminal to a subprocess (`#:inline-output #t`) cannot be repeatable;
passing both flags is an error.

```scheme
(define-command! "delete-line" "Delete the current line; repeatable."
  (lambda (pane)
    (call! "select-line" pane)
    (call! "delete-selection" pane))
  #:repeatable #t)
```

### Typed commands (`:` commands)

The registry also holds the commands run from the `:` command line. They
share one name namespace with key-bound commands, so the two can never
collide.

## Layer 2: Commands

A command implementation is a plain function of the text, its selections,
and whatever else its kind takes (a count, the move/extend mode, a
character). It returns the new state, and knows nothing about keys, modes,
or how it was invoked.

## Layer 3: The Keymap

The keymap is a trie, a tree in which each node stands for one key of a
sequence, mapping key sequences to command names. The value stored at a leaf
is a name, not a function.

That separation is what lets scripts rewrite the keymap. A user binding is
`key → command-name`, and the trie can change without touching any
execution logic.

A node is either:

- a **leaf**: a complete binding, which dispatches the named command;
- an **interior** node: more keys are needed (`m` → `i` → `w` for
  inner-word).

A command that needs a character, such as find (`f`) or replace (`r`), is an
ordinary leaf. The command itself asks for the next key when it runs (see
"Commands that read a character" below), so the trie needs no special node
for it.

### Four keymaps, in layers

HUME keeps one trie per mode:

| Trie | Purpose |
|------|---------|
| Normal | Main keymap for Normal mode |
| Extend | Sparse overrides for Extend mode (checked first) |
| Insert | Single-key bindings for Insert mode |
| Command | Single-key bindings for the `:` prompt |

Each trie holds one layer of bindings per origin: HUME's own defaults, then
plugins, then the user's `init.scm`. The highest layer that knows a sequence
decides it, and a layer can also unbind a sequence, hiding what the layers
below it bind. On top of the global keymap, a buffer can have named keymaps
attached to it, which are looked up first.

The extend trie ships empty, because flipping anchor and head is already
reachable through `Ctrl-e` in both Normal and Extend mode. Any key not found
there falls through to the normal trie with extend mode active. A binding in
the extend trie is a different command for that key in Extend mode: a plugin
that prefers `o` for the flip binds `o` there to the flip command.

## Before dispatch: who owns the keyboard

A key does not always reach the editing surface. HUME keeps a stack of
input handlers, topmost first: the editing surface sits at the bottom, and
above it sits whatever is open, such as a command-line prompt, a search
field, a selection menu, a fuzzy-finder panel or a confirmation question.
Each entry gets the first look at every key, paste and mouse event, and
either handles it, passes it down, or swallows it.

Precedence follows the order things were opened in, not a fixed ranking:
whatever is topmost decides. That is why a stray key typed while an
unrelated question is on screen still runs its own binding (the question
passes it down once it is done with it). It is also why a fuzzy-finder
panel owns the keyboard the moment it opens: it is full-modal and never
passes anything down.

## Layer 4: Dispatch

The dispatcher receives a command name and an extend flag, turns the flag
into the move/extend parameter, looks the command up in the registry, and
calls it.

The flow on a keypress that reaches the editing surface:

```
keypress
  → keymap trie walk     → a command name (e.g. "move-right")
  → dispatcher           → converts extend flag to move/extend mode
  → registry lookup      → the function for "move-right"
  → call                 → function(text, selections, count, mode)
```

### Three ways to get Extend mode

**1. Sticky extend mode.** `e` enters Extend mode, and every command runs
with extend semantics until the mode is left. The extend trie is checked
first for per-key overrides. Acting destructively on the selection (delete,
paste, replace) leaves Extend mode; yank and motions keep it.

**2. Ctrl-key one-shot extend.** On a terminal with the kitty keyboard
protocol, `Ctrl-l` strips the Control modifier, looks up `l` in the normal
trie (`"move-right"`), and dispatches it in extend mode. On a terminal
without the protocol, an unbound Ctrl-letter does nothing.

**3. Force-extend bindings.** A binding can be declared to always extend:
`Ctrl-x` always accumulates line selections, with or without sticky extend
mode or kitty. This works on any terminal.

```scheme
(bind-key! 'normal "f" "select-line")          ; move mode
(bind-key-extend! 'normal "C-f" "select-line") ; always extends
```

The user only ever writes the base command name. The one-shot is automatic
only for an *unbound* Ctrl-letter: binding the key explicitly takes over, so
a binding that should always extend says so with `bind-key-extend!`.

### Counts: a bare keypress versus an explicit count

Dot-repeat and a few other paths care whether a count was typed, not only
which number ends up used. The dispatcher therefore tracks "no count was
typed" separately from "an explicit count of *n*": a bare `w` and `1w` both
move one word, but the layers above dispatch can tell them apart.

Scheme has no built-in way to say "this argument was omitted", so a script
command receives a count of zero for "no count was typed". A command that
forwards its count to another passes the zero along, and the receiver reads
it back as "no count", so a bare keypress stays bare through a chain of
commands calling each other.

### Commands that read a character

Find-forward, find-backward and replace need a character. When one is
dispatched without it, the command asks for the next key, and that key is
read as its character. Everything else about the call, including whether it
extends, was fixed when the triggering key was pressed. A script can pass
the character directly instead, and dot-repeat hands back the one recorded.

## Commands are mode-agnostic

Commands in the registry have no mode affinity. If a script binds
`"flip-selections"` to a key in the insert keymap, the insert handler runs it
through the same dispatch path: the selection flips, and the editor stays in
Insert mode because nothing asked it to leave. Whether that binding is useful
is the user's call.

## Insert mode limitations

The insert keymap binds single keys only: insert mode is for typing, not for
key sequences. Any command can sit on such a key, including one that then
reads a character.

## Independence of layers

Any layer can change without touching the others:

- **New command**: add the function, register it, bind a key. Dispatch is
  unchanged.
- **Rebind a key**: touch only the keymap.
- **Change dispatch** (to add macro recording, say): touch only the
  dispatcher.
- **User keymaps from scripts**: rewrite trie entries. The registry and
  dispatch are unaffected.

The layers communicate only through names, so a change in one cannot corrupt
another.

Dispatch also enforces one invariant at compile time: every native command's
function runs through a single funnel. Each function is stored wrapped in a
type only the funnel can unwrap, so a second call site that tries to invoke
it directly does not compile. The funnel applies the bookkeeping around every
command: dot-repeat, paste-session commits, jump-list updates and the
extend-mode exit. A second path that bypassed it would lose that bookkeeping
without any test of the command's main effect noticing.
