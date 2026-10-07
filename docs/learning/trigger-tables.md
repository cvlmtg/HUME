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
named by strings, so the kind has to travel with the name. Making the kind
part of the value does that:

```
enum Listener { Hook(name), Completion(name) }
```

Two listeners are equal when they are the same kind with the same name. "The
hook called `sig`" and "the source called `sig`" are different values, with
no flag for a key, a lookup or a comparison to forget. Deriving an ordering
gives a deterministic firing order for free: hooks first, then sources, each
by name.

## An invariant belongs to the constructor

A table must never hold a listener with an empty set of characters: such an
entry fires on nothing, yet shows up when you count, list, or clear.

The way to enforce that is to make the bad state impossible to *build*:

```
CharSet::new(chars) -> Option<CharSet>     // None when chars is empty
```

The only way to obtain a character set is a function that refuses to make an
empty one, and that sorts and de-duplicates what it keeps. Setting a
listener's characters then has two outcomes, a stored non-empty set or
a removal, and no other part of the program can get it wrong. The rule is held
by the type, not by every place that writes.

## One structure, addressed by scope

The table is an ordered map from listener to character set, nothing more. A
scope's table holds as many entries as there are features registering
triggers for it, a handful, so asking it a question means reading one small
map and comparing: there is no second structure to keep in step.

Which table to read is part of the address. Tables are keyed by scope: one per
language, and one per buffer's attachment to a server. Answering for a buffer
reads its language's table and its attachments' tables, and the union of what
fires is the answer, each listener once.

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

A completion source also has a numeric id inside the registry. The table
stores the source's name instead. When the configuration is reloaded the
registry is rebuilt and ids are handed out again, so an id kept by a table
that outlives the reload could now name a different source. A stored name that
stops resolving fires nothing. Where a reference can outlive what it
points at, prefer the form whose failure is harmless.
