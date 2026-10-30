# Dot-Repeat: Re-run the Deterministic, Record the Interactive

## Why not replay the changeset

The obvious way to repeat the last edit is to apply its changeset again. But
a changeset is positions in one specific document ("delete characters 12
through 15"), and the cursor has usually moved by the time you press repeat:
the same positions edit the wrong text. (See [Changesets](changesets.md).)

So dot-repeat stores a recipe instead: which command ran, its count, the
selection it acted on, what the user answered when it asked for something,
and, if it opened an editing session, every input that session received.
Replay runs the recipe again at the current cursor, so each step decides
afresh what it applies to.

## What gets recorded

Only a command its author marked repeatable starts a change: delete or paste
make sense at a new cursor, toggling a setting doesn't. See [The
Command/Keymap/Dispatch Architecture](command-keymap-dispatch.md) for how a
command opts in.

Inside the editing session such a command opens, every input is recorded as
it arrives, as what it was: a typed key, a pasted block of text, a bound
command firing on a key. Nothing is inferred afterward by diffing the
buffer. A bound command is replayed by running it again, so it can take a
different branch at the new cursor; that is what makes `.` useful for "add a
character, then skip past the matching bracket" rather than a fixed replay
of its text.

## Questions get their recorded answers

Some commands stop and ask: which character, which item from a list, what to
type into a prompt, which menu entry. Asking again on replay would wait for
nobody, so the recording keeps the user's *answer*, in the order the
questions were asked. Replay re-runs the command and hands each question its
answer instead of waiting, and what the command does with it is decided
again at the new cursor: picking "add an import for this name" adds the
import the new spot needs.

A pick from a list is remembered by the item's text (and which of several
items with that text) when the list is complete as it opens: the re-run
builds its own list, and the item with that text supplies the value for the
new spot. A list whose items arrive later, streamed from a search tool say,
has nothing to match when replay answers it, so its pick is remembered by
the value it handed back.

A question can stay open across keystrokes: its place among the answers is
reserved when it opens and filled when it closes. It belongs to the change
only if a repeatable command's code asked it: the command itself, code its
answers set running, or a key bound in the editing session whose command is
repeatable. A question anything else asks while a change is recorded (a key
bound to a command that isn't repeatable, a hook, a timer) can't be answered
on replay, so it ends the change there.

A list in the drawer is for moving around, not editing, so a pick from it is
never recorded.

## Completion: the one recorded effect

Accepting a completion hands its choice to nothing that could take it again:
the editor writes the text itself. So the recording keeps the net text
around the cursor (how much was removed behind and ahead of it, and what
replaced it), and replay applies that directly. A bound key that accepts a
completion anywhere in its run is recorded as that one edit. An edit the
completion made elsewhere, such as an import at the top of the file, means
nothing at another cursor and is dropped.

## Replay fails loudly

When a re-run takes a different branch (asks for a character where it
picked from a list, asks more questions than were answered, can't find the
picked item, reaches a completion it didn't reach before), replay stops with
an error rather than guess, or open something with nobody there to answer
it. Anything unrelated to text, like a binding that saves the file, runs
again.

---

*See also: [Changesets](changesets.md) for what a changeset contains and
why it's position-dependent; [The Command/Keymap/Dispatch
Architecture](command-keymap-dispatch.md) for the `#:repeatable` gate
commands opt into.*
