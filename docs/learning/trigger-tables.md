# Trigger Tables: Who Fires on Which Character

## The question

Some features react to a single typed character. Signature help wants to open
on `(`, member completion on `.`. A plugin says which characters, and the
editor, on every keystroke in Insert mode, asks one question:

> Which listeners fire on this character, in this buffer?

Everything below is about answering that question with a representation whose
shape matches it.

## Identity is a sum type

A listener is one of two things: a *hook name* that `on-trigger-char` handlers
filter on, or a *completion source* the editor invokes directly. Both are
named by strings, so it is tempting to store the string and a tag saying
which kind it is. Then "the hook called `sig`" and "the source called `sig`"
differ only by a flag that every key, every lookup and every comparison must
remember to carry.

Make the kind part of the value instead:

```
enum Listener { Hook(name), Completion(name) }
```

Two listeners are equal when they are the same kind with the same
name. The compiler enforces what the flag only suggested, and deriving an
ordering gives a deterministic firing order for free: hooks first, then
sources, each by name.

## An invariant belongs to the constructor

A table must never hold a listener with an empty set of characters: such an
entry fires on nothing, yet shows up when you count, list, or clear. The usual
fix is a convention: "an empty set means delete the entry", repeated at every
place that writes.

A stronger fix is to make the bad state impossible to *build*:

```
CharSet::new(chars) -> Option<CharSet>     // None when chars is empty
```

The only way to obtain a character set is a function that refuses to make an
empty one, and that sorts and de-duplicates what it keeps. Setting a
listener's characters then has two outcomes, a stored non-empty set or
a removal, and no other part of the program can get it wrong. The rule has
moved from "everyone must remember" to "the type cannot say otherwise".

## One structure, one source of truth

The table is an ordered map from listener to character set, nothing more. It
is tempting to add an index the other way round, from character to listeners,
because that is the direction of the question. But an index is a second copy
of the same facts. Every write must update both, and a bug in either makes
them disagree with no error anywhere.

The deciding fact is size. A scope's table holds as many entries as there are
features registering triggers for it, a handful. Reading one small table per
keystroke is cheap, and there is nothing to keep in step. An inverted index
earns its keep when the table is large and queries dominate; here it would add
a way to be wrong and no measurable gain.

What did matter was *which* table to read. When every language's entries sit
in one flat map, answering for one buffer means scanning all of them and
discarding most. Keying by language first, and reading only the buffer's own
language table plus the tables of its server attachments, makes the scope part
of the address instead of a filter.

## Testing a data structure against a naive model

How do you know the table behaves? Write the specification the slow way, in a
form too simple to be wrong, and compare. Here the model is a plain list of
`(listener, set)` pairs: setting removes any old pair and appends the new one,
and asking filters the list and sorts the result. The real table is checked
against it after every step of a random sequence of operations, for every
character of a small alphabet.

```
for each random sequence of set / clear operations:
    apply it to the table and to the model
    for each character in the alphabet:
        assert table.fires_on(c) == model.fires_on(c)
```

A small alphabet is deliberate: with few characters and few listeners, sets
overlap and repeat constantly, which is where replace-versus-merge and
empty-set mistakes hide. The model shares no code with the table, so a mistake
in one cannot hide behind the same mistake in the other.

## Names, not numbers, across a reset

A completion source also has a numeric id inside the registry. Storing that id
in the table would be slightly faster, and wrong in a subtle way: when the
configuration is reloaded the registry is rebuilt and ids are handed out
again, so an id kept by a table that outlives the reload could now name a
different source. A stored *name* that no longer resolves simply fires
nothing. Where a reference can outlive what it points at, prefer the form
whose failure is harmless.
