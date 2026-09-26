# Dot-Repeat: Re-run the Deterministic, Record the Interactive

## The problem with replaying a changeset

The obvious way to implement "repeat the last edit" is to remember the
changeset it produced and apply it again. That fails immediately: a
changeset is a set of positions in one specific document — "delete
characters 12 through 15" — and the cursor has usually moved by the time you
press repeat. Applying the same positions at a new cursor either edits the
wrong text or falls off the end of the line.

So dot-repeat doesn't store the *effect* of a command. It stores a recipe
for reproducing it: which command ran, what count and character argument it
took, which selection it acted on, and — if it opened an editing session —
every input that session received, in order. Replaying means running that
recipe again at the current cursor, so each step gets to decide fresh what
it applies to. (See [Changesets](changesets.md) for what a changeset is and
why it's the wrong unit to replay.)

## Two separate questions

"Can this be repeated at all?" and "how is one particular replay carried
out?" are two different gates.

The first is a property of the command itself, decided once by whoever
wrote it: does re-running the whole command body at a new cursor position
even make sense? A command that toggles a setting or shells out to an
external tool has no meaningful answer to "repeat this" — its author simply
never marks it repeatable. A command like delete or paste clearly does. See
[The Command/Keymap/Dispatch Architecture](command-keymap-dispatch.md) for
how a command opts in.

The second question only comes up once a repeatable command has opened an
editing session — typing to insert text, say — and covers something the
first gate says nothing about: each individual input that session received
while it was open gets classified on its own.

## Record what arrived, not what it seemed to do

Every input reaching an open editing session — a typed key, a pasted block
of text, a bound command firing on a key — gets recorded the moment it
arrives, before anything runs. Nothing is inferred afterward by diffing the
buffer or guessing from side effects. That distinction matters: a command
bound to a key can branch on where the cursor happens to be, so replaying it
means running the *command* again, not something that merely looks like
what it did last time. Re-running it lets it take a different branch at the
new cursor, which is exactly the point — that's what makes `.` useful for
things like "add a character then move past matching brackets" rather than
a dumb macro.

## When re-running isn't safe

Some inputs can't be handled that way. If accepting a suggestion from a
completion popup, or picking an item from a list, is itself the input, then
re-running whatever *opened* that popup or list on replay would just open it
again and stop — there's no user sitting there to make the same choice a
second time.

An input like that is called *interactive*: its result depends on a choice
the user made while it was running. It isn't declared as interactive ahead
of time by whoever wrote the command; nothing about the binding itself says
so. It becomes interactive at the moment it actually reaches one of a
handful of operations — opening a completion popup and accepting from it,
opening a picker — during that particular run. A binding that never reaches
one of those operations this time around is not interactive this time,
even if it's capable of being interactive on some other run.

An interactive input is recorded differently: instead of "which binding
ran," it stores the net text that ended up around the cursor once the
choice was made — however many characters were removed behind and ahead of
the cursor, and what replaced them. Replaying it means applying that
recorded text directly, never re-running the input that produced it. If a
single dispatch does several things and any part of it goes interactive,
the entire dispatch collapses into that one recorded edit rather than
splitting into an interactive part and a re-run part — once a choice was
involved anywhere in it, none of it can be safely re-derived at a new
cursor.

## Picking from a list can resolve later

Accepting a completion resolves immediately, in the same dispatch that
triggered it. Picking from a list doesn't have to: the list can stay open
across further keystrokes, and the pick — or a cancellation — only resolves
once it closes. Dot-repeat's recording waits for that: the net edit isn't
finalized until the pick (or dismissal) actually happens, however much later
that turns out to be. If dismissing the list made no edit at all, nothing is
recorded for that step.

## Replay fails loudly on a mismatch

Because an input's interactive/non-interactive status is decided per run
rather than declared up front, replay can in principle reach a binding that
takes a different branch than it did the first time — one that goes
interactive now when it didn't before, say. Rather than silently reopening
a popup or a list with no one there to answer it, that's treated as an
error: replay stops and reports that the recorded input took a different
branch this time, instead of guessing.

## What doesn't come back

Only the text right around the cursor is replayed. An edit somewhere else
in the document — a completion that also inserts an import statement at the
top of the file, alongside the change at the cursor — has no meaning at a
different cursor position, so it's dropped from what's replayed; only the
cursor-adjacent part travels. A side effect with no connection to text at
all, like a binding that saves the file, isn't special-cased either way — it
simply runs again in full, because it was never part of the text-editing
question this whole scheme exists to answer.

---

*See also: [Changesets](changesets.md) for what a changeset contains and
why it's position-dependent; [The Command/Keymap/Dispatch
Architecture](command-keymap-dispatch.md) for the `#:repeatable` gate
commands opt into.*
