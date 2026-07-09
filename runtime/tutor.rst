==============================================================
 HUME Tutor
==============================================================

A hands-on introduction to HUME — Hume's Unfinished Modal Editor.

Welcome. This buffer is your practice space. Every lesson has
exercises with text you are meant to edit. Do so freely — you
cannot break anything here. To reset this buffer to its original
state, close it with ``:bd!`` and re-open it with ``:tutor``.

When you are ready, move to the Introduction below. Press ``j`` to
move down.

To quit at any time, type ``:q`` and press Enter.

Introduction
============

HUME is a modal editor. You spend most of your time in Normal mode,
where keys perform commands rather than typing text. Press ``i`` to
enter Insert mode and type; ``Esc`` to return to Normal mode.

The most important thing about HUME: the cursor IS the selection.
The selection always covers at least one character — the one "under"
the cursor.

This shapes HUME's editing model: SELECT FIRST, ACT SECOND. Make a
selection (navigate to a word, extend to a region, match a pattern),
then apply a command (delete, change, yank). Build the selection,
then act. Every lesson builds on this idea.

Lesson 1 — Moving Around
========================

1.1 Basic Motion
----------------

+---+------------+
| h | move left  |
+---+------------+
| j | move down  |
+---+------------+
| l | move right |
+---+------------+
| k | move up    |
+---+------------+

The arrow keys also work, but ``h j k l`` keep your hands on the home
row. Try them now on the practice text below.

Exercise
~~~~~~~~

Navigate to every word on the line below using ``h``, ``j``, ``k``, and ``l``:

The quick brown fox jumps over the lazy dog.

1.2 Word Motion
---------------

+---+------------------------------------------------------------------+
| w | select the next word (stops at punctuation boundaries)           |
+---+------------------------------------------------------------------+
| W | select the next WORD (whitespace-delimited; ignores punctuation) |
+---+------------------------------------------------------------------+
| b | select the previous word                                         |
+---+------------------------------------------------------------------+
| B | select the previous WORD                                         |
+---+------------------------------------------------------------------+

The difference: in "config.toml log-file", pressing ``w`` repeatedly
selects "config", then ``.``, then "toml", then "log", then ``-``,
then "file" (six steps — dots and dashes are boundaries). Pressing
``W`` instead selects "config.toml", then "log-file" (two
steps — only whitespace divides WORDs).

Exercise
~~~~~~~~

Try ``w`` on the line below and watch it step through each piece.
Then navigate back to the start of the line with ``b`` and try
``W``, then navigate back once more with ``B``.

config.toml log-file backup.zip

1.3 Line Motion
---------------

+----+----------+---------------------------------------+
| gh | or  Home | go to start of line (first character) |
+----+----------+---------------------------------------+
| gs |          | go to first non-blank character       |
+----+----------+---------------------------------------+
| gl | or  End  | go to end of line (last character)    |
+----+----------+---------------------------------------+

To select a range up to the start/end of the line: use
``Ctrl+g h`` / ``Ctrl+g s`` / ``Ctrl+g l`` for a one-shot
extend, or enter Extend mode first (see Lesson 2.2).

Exercise
~~~~~~~~

Navigate to the start and end of this intentionally long line:

The release pipeline failed during artifact signing because the certificate had expired three days before the deployment window.

1.4 Jumping Around
------------------

+----+------------------------------------+
| gg | go to the FIRST line of the buffer |
+----+------------------------------------+
| ge | go to the LAST line of the buffer  |
+----+------------------------------------+

To jump to a specific line by number, type ``:N`` at the command prompt
(for example, ``:42`` lands on line 42). The full name is ``:goto 42``.
Both forms record a jump, so ``Ctrl+o`` brings you back.

+--------+---------------------------------------------+
| Ctrl+o | jump backward through your movement history |
+--------+---------------------------------------------+
| Ctrl+i | jump forward through your movement history  |
+--------+---------------------------------------------+

Opening a file, jumping to search results, or switching buffers all
record an entry. Use ``Ctrl+o`` to retrace your steps — for instance,
to return from a tutor exercise to wherever you were before.

Some Ctrl key combinations — including ``Ctrl+i`` — require the Kitty
keyboard protocol to work correctly. Without it, the terminal cannot
distinguish ``Ctrl+i`` from Tab, so the forward-jump command will not
fire. To check whether the protocol is active, look for the cat
glyph (ᓚᘏᗢ) in the statusline.

Exercise
~~~~~~~~

Press ``gg`` to jump to the top of the tutor, then press ``ge``
to jump to the end, then press ``Ctrl+o`` twice to return here.

Exercise
~~~~~~~~

Type ``:goto 3`` and press ``Enter`` to jump to the line nr. 3 of
this buffer. Then press ``Ctrl+o`` to return here.

1.5 Scrolling
-------------

+----------+-------------------------+
| Ctrl+d   | scroll down half a page |
+----------+-------------------------+
| Ctrl+u   | scroll up half a page   |
+----------+-------------------------+
| PageDown | scroll down a full page |
+----------+-------------------------+
| PageUp   | scroll up a full page   |
+----------+-------------------------+

Scrolling moves the viewport without changing the cursor position.
To reposition the viewport around the cursor:

+----+--------------------------------------+
| zz | center the viewport on the cursor    |
+----+--------------------------------------+
| zt | place the cursor row at the top      |
+----+--------------------------------------+
| zb | place the cursor row at the bottom   |
+----+--------------------------------------+

Exercise
~~~~~~~~

Press ``Ctrl+d`` to scroll down half a page, then ``Ctrl+u`` to
scroll back up. Then press ``zt`` to pull the current line to the
top of the window, ``zz`` to center it, and ``zb`` to drop it to
the bottom.

1.6 Paragraph Motion
--------------------

+---+---------------------------------------------------+
| { | jump to the previous blank line (paragraph start) |
+---+---------------------------------------------------+
| } | jump to the next blank line (paragraph end)       |
+---+---------------------------------------------------+

Exercise
~~~~~~~~

Press ``}`` twice to jump two paragraphs forward, then press ``{``
twice to jump back.

1.7 Count Prefixes
------------------

Prefix a motion with digits to repeat it:

+----+--------------------------+
| 3w | step three words forward |
+----+--------------------------+
| 5j | move five lines down     |
+----+--------------------------+
| 2{ | jump two paragraphs back |
+----+--------------------------+

Counts apply to motions, not edits.

Exercise
~~~~~~~~

Place the cursor on "January" in the line below, then press ``3w``
to jump three words forward — you should land on "April".
Then press ``2b`` to jump two words backward — you should land on "February":

January February March April May June

Summary
-------

+-----------------+---------------------------------------------+
| h / j / k / l   | basic motion                                |
+-----------------+---------------------------------------------+
| w / W           | next word / WORD                            |
+-----------------+---------------------------------------------+
| b / B           | previous word / WORD                        |
+-----------------+---------------------------------------------+
| gg / ge         | first / last line                           |
+-----------------+---------------------------------------------+
| :N / :goto N    | jump to line N by number (e.g. ``:42``)     |
+-----------------+---------------------------------------------+
| gh / gl / gs    | line start / end / first non-blank          |
+-----------------+---------------------------------------------+
| { / }           | paragraph motion                            |
+-----------------+---------------------------------------------+
| Ctrl+d / Ctrl+u | scroll half-page down / up                  |
+-----------------+---------------------------------------------+
| zz / zt / zb    | center / top / bottom viewport              |
+-----------------+---------------------------------------------+
| Ctrl+o / Ctrl+i | jump backward / forward in movement history |
+-----------------+---------------------------------------------+
| 3w / 5j         | count + motion                              |
+-----------------+---------------------------------------------+

Lesson 2 — Selections
=====================

In HUME, a selection is a contiguous region with an anchor (fixed end)
and a head (moving end). The cursor sits on the head character. This
lesson covers the tools for building and shaping a single selection.
Editing what you have selected — deleting, changing, replacing — comes
in Lesson 3.
HUME can also handle multiple selections. They will be covered in Lesson 8.

2.1 Line Selection
------------------

+--------+----------------------------------------------------------+
| x      | select the current line (including the newline)          |
+--------+----------------------------------------------------------+
| X      | select the line in the backward direction                |
+--------+----------------------------------------------------------+
| Ctrl+x | like  x , but EXTENDS (accumulates) rather than replaces |
+--------+----------------------------------------------------------+

Repeated ``x`` walks the selection down through multiple lines.

Exercise
~~~~~~~~

Navigate onto the first line of the exercise, press ``x`` to select
it, then press ``Ctrl+x`` twice to extend to all three lines.
Observe the selection spans all three lines. Now press ``X`` twice —
the selection shrinks back up one line at a time instead of growing
further. The opposite key reverses direction.

import os
import sys
import json

2.2 Extend Mode
---------------

+---+--------------------+
| e | toggle Extend mode |
+---+--------------------+

In Extend mode, every motion ADDS to the selection rather than
replacing it. Press ``e`` again to leave Extend mode. Check the
bottom-right corner of the window — it shows the current mode
("EXT" when active, "NOR" when not).

Selection-consuming edits — delete, change, paste, replace — exit
Extend mode automatically and return you to Normal. Yank (``y``) keeps
you in Extend mode so you can extend further before acting.

Motions run backward too: moving toward where you started shrinks the
selection instead of growing it. ``w``/``b`` and ``x``/``X`` shrink one
whole word or line at a time, and the word or line you started on
always stays selected — pressing past it flips the selection to grow
in the other direction instead of cutting it off partway.

Exercise
~~~~~~~~

Press ``e`` to enter Extend mode, then press ``w`` several times to
grow the selection across multiple words. Observe the span, then
press ``b`` a few times to shrink it back down word by word, watching
it contract to the word you started on. Press ``;`` to collapse it —
you land back in Normal mode automatically:

The build finished in under two seconds on the CI server.

One-shot Extend with Ctrl
~~~~~~~~~~~~~~~~~~~~~~~~~

You can extend the selection for a single motion without entering
Extend mode by holding Ctrl. Every motion supports it:

+-----------+--------------------------------------------------------+
| Ctrl+w    | extend to the next word (parallel to ``e`` then ``w``) |
+-----------+--------------------------------------------------------+
| Ctrl+f<c> | extend forward to <char> (inclusive)                   |
+-----------+--------------------------------------------------------+
| Ctrl+t<c> | extend forward to just before <char>                   |
+-----------+--------------------------------------------------------+
| Ctrl+F<c> | extend backward to <char>                              |
+-----------+--------------------------------------------------------+
| Ctrl+T<c> | extend backward to just after <char>                   |
+-----------+--------------------------------------------------------+
| Ctrl+g h  | extend to start of line                                |
+-----------+--------------------------------------------------------+
| Ctrl+g l  | extend to end of line                                  |
+-----------+--------------------------------------------------------+
| Ctrl+g s  | extend to first non-blank character                    |
+-----------+--------------------------------------------------------+

The idiomatic delete-to-char pattern: press ``Ctrl+f.`` to extend the
selection to the next period, then act on the span (``d`` to delete,
``c`` to change, etc. — see Lesson 3). The ``f``/``t`` find commands
are taught in full in Lesson 6.

Note
~~~~

One-shot Ctrl extend requires the kitty keyboard protocol
(look for the cat glyph ᓚᘏᗢ in the statusline). Without it,
Ctrl+w / Ctrl+f / etc. do nothing — use Extend mode instead.
The exercises below are labelled (Kitty) for terminals with
the protocol and (Legacy) for those without.

Exercise
~~~~~~~~

(Kitty) Press ``Ctrl+w`` a few times to grow the selection across
words below, then press ``;`` to collapse it.
(Legacy) Press ``e`` then ``w`` repeatedly to the same effect.

the pipeline runs tests before merging any pull request

Exercise
~~~~~~~~

(Kitty) Press ``Ctrl+f,`` to extend the selection to the comma
below, then press ``;`` to collapse it.
(Legacy) Press ``e`` then ``f,`` to the same effect.

The server starts on port 8080, then waits for connections.

2.3 Collapsing and Flipping
---------------------------

+--------+---------------------------------------------------------+
| ;      | collapse the selection to a single-char at HEAD         |
+--------+---------------------------------------------------------+
| Ctrl+; | collapse the selection to a single-char at ANCHOR       |
+--------+---------------------------------------------------------+

Both ``;`` and ``Ctrl+;`` also exit Extend mode if it is active.

Note: ``Ctrl+;`` requires the kitty keyboard protocol.

Support for this protocol varies between terminals. Some implement only
part of it, some have rough edges, and some need it enabled in their
configuration. If ``Ctrl+;`` does nothing in your terminal, check its
documentation for keyboard-protocol settings — or fall back to the
plain ``;`` command above.

Flipping the Selection
~~~~~~~~~~~~~~~~~~~~~~

+--------+---------------------------------------------------------+
| o      | (in Extend mode) swap anchor and head                   |
+--------+---------------------------------------------------------+
| Ctrl+e | swap anchor and head — works in Normal and Extend mode, |
|        | and works on all terminals.                             |
+--------+---------------------------------------------------------+

Flipping moves the cursor to the other end of the selection without
changing what is selected. This matters because ``;`` collapses to the
cursor's end, and the next plain motion starts from there — flip first
when you want to keep the opposite end.
Shrinking from the cursor's end is just a backward motion, as in
Lesson 2.2. Flip first when you want to grow or shrink from the other
end instead.

Exercise
~~~~~~~~

Press ``w`` to select the word below, then press ``Ctrl+e`` to flip
— the cursor jumps from the end of the word to the start. Press
``;`` to collapse back to a single character. Notice how the cursor
is now on the first letter of the word instead of the last:

rename this variable

Summary
-------

+------------------+---------------------------------------+
| x / X           | select line / select line backward    |
+------------------+---------------------------------------+
| Ctrl+x           | extend line selection                 |
+------------------+---------------------------------------+
| e                | toggle Extend mode                    |
+------------------+---------------------------------------+
| ;                | collapse to head                      |
+------------------+---------------------------------------+
| Ctrl+;           | collapse to anchor                    |
+------------------+---------------------------------------+
| o                | flip anchor ↔ head (in Extend mode)   |
+------------------+---------------------------------------+
| Ctrl+e           | flip anchor ↔ head (any mode)         |
+------------------+---------------------------------------+

``Ctrl+w`` / ``Ctrl+f<c>`` / ``Ctrl+t<c>`` — one-shot extend (kitty only)
``Ctrl+g h`` / ``Ctrl+g l`` / ``Ctrl+g s`` — one-shot extend line motions

Lesson 3 — Editing with Selections
===================================

Every action in HUME consumes the current selection. The single
character under your cursor is always selected — you can act on it
immediately without making an additional selection first.

3.1 Delete
----------

+---+------------------------------+
| d | delete the current selection |
+---+------------------------------+

With a fresh 1-char selection, pressing ``d`` deletes the character
under the cursor.

Exercise
~~~~~~~~

Delete every "@" prefix marker in the line below using ``d``:

The @quick @brown @fox

Exercise
~~~~~~~~

Now press ``w`` to select the duplicate word "file", then ``d`` to
delete it. Press ``d`` again to delete the extra space remaining:

The configuration file file needs to be updated.

Exercise
~~~~~~~~

To delete a WHOLE LINE: press ``x``, then ``d``.
Delete the debug statement in the middle:

server.start()
print("DEBUG: server object:", server)
server.listen(8080)

3.2 Change
----------

+---+--------------------------------------------+
| c | delete the selection and enter Insert mode |
+---+--------------------------------------------+

Exercise
~~~~~~~~

Navigate to "yesterday" using ``w``, then press ``c`` and type
"Monday", then press ``Esc``:

The deadline was yesterday.

3.3 Replace
-----------

+---------+------------------------------------------------------+
| r<char> | replace every character in the selection with <char> |
+---------+------------------------------------------------------+

Unlike ``c``, replace stays in Normal mode — no Insert, no ``Esc``
needed. On a 1-char selection it swaps the single character under
the cursor. On a wider selection it overwrites every character in
the range (newlines are preserved to keep line structure intact).

Exercise
~~~~~~~~

Navigate onto the "O" below (use ``j`` to go to the correct line,
then press ``gl`` to move to the last character of the line) and
press ``r0`` to fix the typo (the letter O was typed instead of
a zero):

Listening on port 808O

Exercise
~~~~~~~~

Press ``w`` to select the word "secret", then press ``r*`` to
mask every character with an asterisk:

password = secret

3.4 Join Lines
--------------

+---+-------------------------------------+
| J | join the current line with the next |
+---+-------------------------------------+

After joining, the cursor lands on the inserted space. If the
selection spans multiple lines, all of them are collapsed
into a single line.

Exercise
~~~~~~~~

Press ``J`` on the first line below to join the two halves into
one sentence:

The deployment failed
due to a missing environment variable.

3.5 Undo and Redo
-----------------

+---+------+----------------------+
| u | undo |                      |
+---+------+----------------------+
| U | redo | (Ctrl+r also redoes) |
+---+------+----------------------+

Exercise
~~~~~~~~

Delete the word "deprecated" below with ``w d``. Press ``u`` to
undo — the word reappears. Press ``U`` to redo — it is deleted
again:

The deprecated function should be replaced.

Summary
-------

+---------+---------------------------------------------------+
| d       | delete selection                                  |
+---------+---------------------------------------------------+
| c       | change selection                                  |
+---------+---------------------------------------------------+
| r<char> | replace each selected char (stays in Normal mode) |
+---------+---------------------------------------------------+
| J       | join current line with the next (or all selected) |
+---------+---------------------------------------------------+
| u       | undo                                              |
+---------+---------------------------------------------------+
| U       | redo                                              |
+---------+---------------------------------------------------+

Lesson 4 — Inserting Text
=========================

To type text you must enter Insert mode. There are several ways to
start inserting, each placing the cursor at a slightly different spot.

+---+-------------------------------------------------------+
| i | insert at the START of the current selection          |
+---+-------------------------------------------------------+
| a | insert at the END of the current selection            |
+---+-------------------------------------------------------+
| I | insert at the beginning of the line (first non-blank) |
+---+-------------------------------------------------------+
| A | insert at the end of the line                         |
+---+-------------------------------------------------------+
| o | open a new line BELOW the cursor and enter Insert     |
+---+-------------------------------------------------------+
| O | open a new line ABOVE the cursor and enter Insert     |
+---+-------------------------------------------------------+

Remember: ``Esc`` exits Insert mode.

4.1 Basic Insert
----------------

Exercise
~~~~~~~~

Select "Name:" with ``W`` or select ":" with ``w`` and press
``a`` to append. Type your name, then press ``Esc``:

Name:

Exercise
~~~~~~~~

Use ``I`` to prepend "DONE: " to the line below (press ``I``,
type the prefix, then ``Esc``):

finish this task

4.2 Open New Lines
------------------

Exercise
~~~~~~~~

Press ``o`` on the line below, then type "Shopping list item 2".
Press ``Esc`` when done:

Shopping list item 1:

Exercise
~~~~~~~~

Press ``O`` on the line below, then type "Shopping list:" as a
title. Press ``Esc`` when done:

First item of the list.

Summary
-------

+-------+------------------------------------+
| i / a | insert at start / end of selection |
+-------+------------------------------------+
| I / A | insert at start / end of the line  |
+-------+------------------------------------+
| o / O | open new line below / above        |
+-------+------------------------------------+

Lesson 5 — Yank, Paste, and the Kill Ring
=========================================

HUME has two stores for copied text: the CLIPBOARD (shared with
the system) and the KILL RING (internal to the editor).

+---+----------------------------------------------------------+
| y | yank (copy) the selection to the clipboard and kill ring |
+---+----------------------------------------------------------+
| p | smart-paste after the selection                          |
+---+----------------------------------------------------------+
| P | smart-paste before the selection                         |
+---+----------------------------------------------------------+

5.1 Yank and Paste
------------------

``y`` copies the selection but leaves it selected. Because ``p``
pastes OVER a multi-character selection (replacing it), pressing
``p`` right after ``y`` silently overwrites the selection with an
identical copy — it looks like nothing happened.

To paste a separate copy, collapse the selection to a single character
first with ``;``, then press ``p``. After the first paste, each
further ``p`` stacks another copy adjacent to it.

``d`` and ``c`` remove their text, so the selection is already
collapsed when you paste — only after ``y`` you need the
explicit ``;``.

Exercise
~~~~~~~~

Navigate to "cache", yank it with ``w y``. Press ``;`` to collapse
the selection, then ``p`` to paste a copy after it. Press ``p`` again
to add a second copy:

The build cache speeds up compilation dramatically.

Exercise
~~~~~~~~

Select the line below with ``x``, yank with ``y``. Press
``;`` to collapse the selection, then ``p`` to paste a duplicate
line below:

server.port = 8080

When the cursor covers a single character, ``p`` inserts after it.
When the yanked text was a whole line (selected with ``x``),
``p`` inserts a new line below.

5.2 The Kill Ring
-----------------

``c`` and ``d`` (change and delete) push their result into the kill
ring separately from the clipboard. The ring keeps the 10 most recent
kills; the oldest drops off when an eleventh arrives.

+---+-------------------------------------------------+
| [ | cycle to the older kill-ring entry and paste it |
+---+-------------------------------------------------+
| ] | cycle to the newer kill-ring entry and paste it |
+---+-------------------------------------------------+

``[`` and ``]`` only work right after a paste — while the paste is
still "live". Pressed otherwise they do nothing. The whole
paste-and-cycle sequence collapses into one undo step: a single
``u`` reverts all of it.

Exercise
~~~~~~~~

Delete the word "stale" below with ``w d``, then delete "unused"
with ``w d``. Now press ``P`` — smart-paste gives you the last kill.
Press ``[`` to cycle to the older kill-ring entry:

Rename the stale and unused methods before the review.

Whitespace kills do not pile up. When the newest kill is nothing but
whitespace — a stray space, a tab, a blank line — the next delete,
change, or yank overwrites it instead of stacking on top. Tidy-up
edits like removing a doubled space therefore never bury the kills you
want to cycle back to. The whitespace is still there to paste right
after you cut it; it only disappears once the next kill arrives.

Exercise
~~~~~~~~

Delete the duplicate word "form" with ``w d``, then press ``d``
once more to remove the leftover space — that space is now the
newest kill. Delete "draft" with ``w d``: it overwrites the space
instead of stacking on top. Press ``P`` to paste "draft", then
``[`` — you cycle straight back to "form", with no throwaway space
in between:

Submit the form form draft today.

5.3 Smart Paste
---------------

``p`` is context-aware:

- after ``c`` or ``d``, it reads from the kill ring
- otherwise it reads from the clipboard

This means you can delete something with ``d`` and immediately ``p``
to paste the deleted text, without switching registers manually.

If the clipboard is empty when ``p`` would read it, it falls back to
the most recent kill so there is always something to paste.

Summary
-------

+-------+-------------------------------+
| y     | yank to clipboard + kill ring |
+-------+-------------------------------+
| c / d | push to kill ring (10)        |
+-------+-------------------------------+
| p / P | paste after / before          |
+-------+-------------------------------+
| [ / ] | cycle ring (after paste)      |
+-------+-------------------------------+

``p`` reads from kill ring after ``c`` / ``d``, from clipboard
otherwise. ``y`` leaves the selection — press ``;`` to collapse
before pasting a copy.

Lesson 6 — Find, Till, and Repeat
=================================

6.1 Find
--------

+---------+-------------------------------------------------------+
| f<char> | move the selection onto the next occurrence of <char> |
+---------+-------------------------------------------------------+
| F<char> | move backward onto the previous <char>                |
+---------+-------------------------------------------------------+
| t<char> | move to just BEFORE the next <char>                   |
+---------+-------------------------------------------------------+
| T<char> | move to just AFTER the previous <char>                |
+---------+-------------------------------------------------------+

These are single-key commands followed by one character. The selection
lands on (or just before/after) the target — a fresh 1-char selection.
To select a range up to a char: use ``Ctrl+f<c>`` / ``Ctrl+t<c>`` for
a one-shot extend, or enter Extend mode first (see Lesson 2.2).

Find and till search *only the current line* — they stop at the end
of the line and never jump to another line. If the character isn't on
this line, the selection stays put. To find across lines, use search
(Lesson 7).

Exercise
~~~~~~~~

Press ``f.`` (find dot) to jump the selection onto the dot in
"config.toml" below. The selection shrinks to just the dot.

Update the config.toml file with the new timeout value.

Exercise
~~~~~~~~

Press ``t,`` (till comma) to land just before the comma below:

Restart the service, then check the logs for errors.

6.2 Repeating Find
------------------

+---+------------------------------------------+
| = | repeat the last find/till forward        |
+---+------------------------------------------+
| - | repeat the last find/till backward       |
+---+------------------------------------------+

Exercise
~~~~~~~~

Press ``f-`` to find the first hyphen below, then ``=`` repeatedly
to advance through each one:

well-known open-source command-line text-editor

6.3 Repeating the Last Action
-----------------------------

+---+----------------------------------------------------------------+
| . | repeats the last edit action (delete, change, insert sequence) |
+---+----------------------------------------------------------------+

Important: ``.`` replays the edit together with any whole-line select
(``x``/``X``) or extend steps you used to build the selection — but
it does **not** replay a word, find, or navigation motion. So after
an edit that followed a motion like ``w``, move and re-select manually
before pressing ``.`` (as shown below).

The idiomatic pattern for repeating an edit across several words:

- ``w`` to select the next word.
- ``c`` to change it.
- ``Esc`` to exit insert mode.
- ``w`` to select the next word.
- ``.`` to repeat the change on the new selection
- ...and so on.

Exercise
~~~~~~~~

Change the first "migrate" to "update" below, then navigate to
the next "migrate" with ``w`` and press ``.`` to repeat the change:

Migrate the schema, migrate the tests, migrate the docs.

Summary
-------

+-------------+-------------------------------------------+
| f<c> / F<c> | find char forward / backward (inclusive)  |
+-------------+-------------------------------------------+
| t<c> / T<c> | till char forward / backward (exclusive)  |
+-------------+-------------------------------------------+
| = / -       | repeat find forward / backward            |
+-------------+-------------------------------------------+
| .           | repeat last selection + edit (not motion) |
+-------------+-------------------------------------------+

Lesson 7 — Search and Text Objects
===================================

7.1 Search
----------

+---+------------------------------------------------------------+
| / | search forward (opens a prompt; type pattern, press Enter) |
+---+------------------------------------------------------------+
| ? | search backward                                            |
+---+------------------------------------------------------------+
| n | jump to the next match                                     |
+---+------------------------------------------------------------+
| N | jump to the previous match                                 |
+---+------------------------------------------------------------+
| * | search the whole word under the cursor                     |
+---+------------------------------------------------------------+

The selection lands on the match. Press ``n`` to advance.

+---------+----------------------------------------------------------+
| Ctrl+/  | use the selected text, literally, as the search pattern  |
+---------+----------------------------------------------------------+

Unlike ``*``, this does not expand to a whole word and adds no
word-boundary anchors — it searches for exactly the text you selected,
including as a substring of other words. Requires a terminal with the
kitty keyboard protocol.

Exercise
~~~~~~~~

Search for "error" below typing ``/error`` then ``Enter``:

The linter found an error on line 12 and another error on line 47.

To clear the search highlights, press ``Esc``. The highlights
disappear, but the pattern is remembered — pressing ``n`` or ``N``
brings it back.

Exercise
~~~~~~~~

Navigate onto the word "warning" below and press ``*`` to search
for it. Press ``n`` to jump to the next "warning", then ``n``
again for the third one:

We saw a warning in the dev build, a warning in the staging run,
and a warning in the production log.

``n`` and ``N`` can also extend the selection instead of just moving it:
``Ctrl+n`` jumps to the next match while keeping the anchor where it is,
growing the selection to cover everything in between, and ``Ctrl+N``
does the same backward. This is a one-shot extend — no need to enter
Extend mode first. Requires a terminal with the kitty keyboard protocol.

Exercise
~~~~~~~~

Navigate onto "risk" below and press ``*`` to search for it, then
press ``Ctrl+n`` twice to grow the selection across all three matches,
covering everything from the first "risk" to the last:

The audit flagged risk in module A, risk in module B, and risk in module C.

Exercise
~~~~~~~~

Unlike ``*``, ``Ctrl+/`` searches any substring, not just whole
words — useful for surveying every function that shares a prefix.
Navigate onto the "p" of "parse_header" below, press ``Ctrl+f_`` to
extend the selection through the underscore ("parse_"), then
``Ctrl+/`` and ``n`` to jump through the other two:

fn parse_header(input: &str) -> Header {
fn parse_body(input: &str) -> Body {
fn parse_footer(input: &str) -> Footer {

7.2 Text Objects
----------------

Text objects select structured regions. Prefix ``mi`` for INNER
(without delimiters) or ``ma`` for AROUND (including delimiters):

+----------+-------------------------------------------+
| mi(  ma( | inner / around parentheses                |
+----------+-------------------------------------------+
| mi[  ma[ | inner / around brackets                   |
+----------+-------------------------------------------+
| mi{  ma{ | inner / around braces                     |
+----------+-------------------------------------------+
| mi"  ma" | inner / around double-quoted              |
+----------+-------------------------------------------+
| mi'  ma' | inner / around single-quoted              |
+----------+-------------------------------------------+
| mi`  ma` | inner / around backtick                   |
+----------+-------------------------------------------+
| mil  mal | inner / around line (incl. \n)            |
+----------+-------------------------------------------+
| mia  maa | inner / around argument (comma-separated) |
+----------+-------------------------------------------+
| miw  maw | inner / around word                       |
+----------+-------------------------------------------+
| miW  maW | inner / around WORD                       |
+----------+-------------------------------------------+

Since ``miw`` and ``miW`` are very frequent operations, HUME adds
a couple of shortcuts: ``mm`` for the inner word and ``MM`` for the
inner WORD.

Exercise
~~~~~~~~

Delete the arguments inside the parentheses below using ``mi( d``:

Call connect(host, port) to open the socket.

Exercise
~~~~~~~~

Select around the string literal (including quotes) using ``ma"``,
then change it with ``c``.

status = "pending"

7.3 Surrounding Delimiters
--------------------------

+----------+------------------------------------------------------------+
| ms<char> | select the TWO surrounding delimiter characters as cursors |
+----------+------------------------------------------------------------+

This selects both the opening AND closing delimiter, giving you two
independent cursors. You can then delete both with ``d``, or change
them with ``r`` + a new delimiter character.

Note: if you have the ``core:helix-surround`` plugin loaded, the
``ms`` binding has different semantics — consult that plugin's docs.

Exercise
~~~~~~~~

Navigate into the parentheses below and press ``ms(`` to select
both delimiters, then ``d`` to delete them (the parens, not the
content). Then press ``,`` to collapse all the cursors back to one:

return (value + offset)

Summary
-------

+---------------+-----------------------------------------+
| / ? n N       | search forward / backward / next / prev |
+---------------+-----------------------------------------+
| *             | search word under cursor                |
+---------------+-----------------------------------------+
| Ctrl+/        | search selection literally              |
+---------------+-----------------------------------------+
| Ctrl+n / N    | extend to next / previous match         |
+---------------+-----------------------------------------+
| mi<d> / ma<d> | inner / around text object              |
+---------------+-----------------------------------------+
| ms<d>         | select surrounding delimiter pair       |
+---------------+-----------------------------------------+

Lesson 8 — Multi-Selection
==========================

HUME can hold many independent cursors simultaneously. This lesson
covers how to create and manage them.

8.1 Pruning Selections
----------------------

+--------+---------------------------------------------------------+
| ,      | keep only the PRIMARY selection (discard all others)    |
+--------+---------------------------------------------------------+
| Ctrl+, | remove the PRIMARY selection (the next becomes primary) |
+--------+---------------------------------------------------------+

Note: ``Ctrl+,`` requires the kitty keyboard protocol. If it does
nothing in your terminal, check its documentation for keyboard-protocol
settings (see also the note in Lesson 2.3) — or fall back to ``,``.

The rest of this lesson creates many simultaneous cursors. ``,`` is the
one key that always returns you to a single selection — learn it first,
and lean on it throughout.

Exercise
~~~~~~~~

Select all three lines below with ``x Ctrl+x Ctrl+x``, then press ``J``
to join them into one line. Press ``,`` to collapse back to a single
selection:

Clone the repo,
install dependencies,
and run the tests.

8.2 Select All
--------------

+---+------------------------------------------------+
| % | select the entire buffer as a single selection |
+---+------------------------------------------------+

This is the starting point for many multi-cursor workflows: select
everything, then narrow it down with a pattern.

Exercise
~~~~~~~~

Press ``%`` and observe the selection covers every character. Then
press ``;`` to collapse back to a single-character selection. Press
``Ctrl+o`` to jump back to where you were before ``%``.

8.3 Select Within
-----------------

+---+-------------------------------------------------------+
| s | select regex matches within the current selection(s)  |
+---+-------------------------------------------------------+

Each match within the selection becomes its own selection. Works
on any selection, not just the whole buffer.

Exercise
~~~~~~~~

Press ``%`` to select all, then ``s`` and type "FIXME" and press
Enter to put a selection on every occurrence, then press ``,`` to
collapse all the selections back to one:

The first FIXME is in the handler, a second FIXME is in the parser,
and a third FIXME in the tests.

The canonical multi-cursor entry: ``%`` (select all) → ``s<pattern>``
(select matches) → edit (applies to all cursors simultaneously).

8.4 Split into Lines
--------------------

+---+-------------------------------------------------------+
| S | split a multi-line selection into one cursor per line |
+---+-------------------------------------------------------+

Each line in the selection becomes its own cursor. Single-line selections
are unchanged. Where ``s`` selects by content (a pattern), ``S`` splits by
structure — one piece per line.

Exercise
~~~~~~~~

Navigate onto the first line below, press ``Ctrl+x`` three times to
select all three lines, then press ``S`` to get one cursor per line.
Press ``c``, type replacement text and ``Esc`` — the edit applies
independently on each line:

lint: skipped
test: skipped
build: skipped

8.5 Trim Selections
-------------------

+---+----------------------------------------------------------+
| _ | trim whitespace from the start and end of each selection |
+---+----------------------------------------------------------+

Exercise
~~~~~~~~

Select the three lines below with ``Ctrl+x`` three times, press ``S``
to split into one cursor per line, then press ``_`` to trim the
trailing whitespace from each line at once:

title = "My Application"   
version = "1.0.0"  
license = "MIT"   

8.6 Cycling the Primary Selection
----------------------------------

+---+----------------------------------+
| ( | cycle primary selection backward |
+---+----------------------------------+
| ) | cycle primary selection forward  |
+---+----------------------------------+

After multi-cursor operations, you can walk through which cursor is
"primary" (the one that anchors pastes, messages, etc.).

Exercise
~~~~~~~~

Select the following lines with ``x`` and then ``Ctrl+x``, then
press ``s`` and type "FIXME". Use ``(`` and ``)`` to cycle the
primary selection. Press ``,`` when done.

The first FIXME is in the handler,
a second FIXME is in the parser,
and the last FIXME in the tests.

8.7 Copy Selection on Next Line
-------------------------------

+---+---------------------------------------------------------------------+
| C | duplicate the current selection on the NEXT line at the same column |
+---+---------------------------------------------------------------------+

This is useful for block editing — duplicate a cursor down through
several lines, then make the same edit on all of them at once.

Exercise
~~~~~~~~

Put the cursor on ``old`` in the first line below and press ``C``
twice to put a cursor on the next two lines. Press ``mm`` to select
the word ``old`` under each cursor, then press ``c``, type "new",
and ``Esc``. All three hostnames update:

old-server-1.example.com
old-server-2.example.com
old-server-3.example.com

8.8 Align Selections
--------------------

+---+---------------------------------------------------------+
| & | align all cursors to the column of the primary's anchor |
+---+---------------------------------------------------------+

Spaces are inserted or removed at the left edge of each non-primary selection
until it sits in the same column as the primary. Multi-line selections are
left unchanged.

Exercise
~~~~~~~~

Go to the first line below, press ``C C`` to create a multi selection,
then press ``w`` to select the three ``=``. Now press ``&`` to align them
all to the primary's (last ``=``) column:

x = 2
y = 3
longer_name = 1

To right-align instead: rotate the primary to the widest item, then flip
all selections so each anchor sits on the RIGHT edge of its match. ``&`` then
aligns right edges.

Exercise
~~~~~~~~

Select the lines below with ``Ctrl+x Ctrl+x Ctrl+x``, then press ``s`` and type
``\d+``, then press ``Enter`` to put a cursor on each number. Press ``)`` to rotate
the primary to "1000". Press ``Ctrl+e`` to flip all selections (anchor moves
to the last digit of each number). Press ``&`` to right-align all numbers:

price: 5
price: 1000
price: 42

Exercise
~~~~~~~~

Select the lines below with ``Ctrl+x Ctrl+x Ctrl+x``, then press ``s`` and type
``=|//`` to select the equal signs and the comments. Press ``Enter`` and then
press ``&`` to align the text, finally press ``,`` to discard all the secondary
selections.

const bananas = 4; // 4 bananas
const apples = 123; // 123 apples
const watermelons = 2; // 2 watermelons

Summary
-------

+--------+-------------------+--------+--------------------+
| ,      | keep primary only | Ctrl+, | remove primary     |
+--------+-------------------+--------+--------------------+
| %      | select all        | s      | select within      |
+--------+-------------------+--------+--------------------+
| S      | split on newlines | _      | trim whitespace    |
+--------+-------------------+--------+--------------------+
| ( / )  | cycle primary     | C      | copy on next line  |
+--------+-------------------+--------+--------------------+
| &      | align selections  |        |                    |
+--------+-------------------+--------+--------------------+

Lesson 9 — Files and Commands
=============================

Typed commands begin with ``:`` followed by a name (and optionally an
argument), then ``Enter``. Many have short aliases.

9.1 Saving and Quitting
-----------------------

+-----------+-----------------------------------------------------------------+
| :w        | save (write) the current buffer to its file                     |
+-----------+-----------------------------------------------------------------+
| :w <path> | save to a different path (save-as)                              |
+-----------+-----------------------------------------------------------------+
| :q        | close the current buffer; quit when no real file buffers remain |
+-----------+-----------------------------------------------------------------+
| :q!       | close and discard unsaved changes                               |
+-----------+-----------------------------------------------------------------+
| :wq       | save and quit                                                   |
+-----------+-----------------------------------------------------------------+
| :qa       | quit the editor (refuses if any buffer is unsaved; use :qa!)    |
+-----------+-----------------------------------------------------------------+

The tutor is opened as a sandboxed copy in a temporary directory
(something like ``/tmp/hume-1234/tutor.rst``), so ``:w`` saves only to
that copy — the installed ``runtime/tutor.rst`` is never touched.
To get a fresh tutor, close this buffer with ``:bd!`` then reopen
with ``:tutor``.

Exercise
~~~~~~~~

Try ``:w`` now to confirm the path in the statusline is a temporary path,
not your runtime directory.

9.2 Opening Files and Reloading
-------------------------------

+-----------+------------------------------------------------------+
| :e <path> | open a file (creates a new buffer)                   |
+-----------+------------------------------------------------------+
| :e        | reload the current file from disk (prompts if dirty) |
+-----------+------------------------------------------------------+
| :e!       | reload and DISCARD unsaved changes                   |
+-----------+------------------------------------------------------+

If you open a path that is already in a buffer, ``:e`` switches to that
buffer instead of opening a duplicate.

9.3 Managing Buffers
--------------------

+------------+---------------------------------------------------+
| :ls        | list all open buffers                             |
+------------+---------------------------------------------------+
| :bnext :bn | switch to the next buffer                         |
+------------+---------------------------------------------------+
| :bprev :bp | switch to the previous buffer                     |
+------------+---------------------------------------------------+
| :b <name>  | switch to a buffer by name, path prefix, or index |
+------------+---------------------------------------------------+
| :bd        | close (delete) the current buffer                 |
+------------+---------------------------------------------------+
| :bd!       | close even if unsaved                             |
+------------+---------------------------------------------------+

In any typed command that takes a path, ``%`` expands to the current
file's path and ``#`` expands to the alternate (previously focused)
file's path. For example: ``:e #`` reopens the alternate buffer.

9.4 Other Useful Commands
-------------------------

+----------------+---------------------------------------------------------+
| :version :ver  | show the editor version                                 |
+----------------+---------------------------------------------------------+
| :messages :mes | review the message and error log                        |
+----------------+---------------------------------------------------------+
| :theme <name>  | load a theme (dark / light / sand / gruvbox)            |
+----------------+---------------------------------------------------------+
| :reload-config | reload the editor configuration file without restarting |
+----------------+---------------------------------------------------------+
| :pwd           | print the working directory                             |
+----------------+---------------------------------------------------------+
| :cd <path>     | change the working directory                            |
+----------------+---------------------------------------------------------+

Exercise
~~~~~~~~

Try ``:version`` to confirm which build you are running.

Exercise
~~~~~~~~

Type ``:theme`` followed by a space, then press ``Tab`` to see the list of
available themes. Keep pressing ``Tab`` to cycle through them. Press
``Esc`` to dismiss the prompt without applying a change.

Summary
-------

+----------------+------------------------+
| :e <path>      | open file              |
+----------------+------------------------+
| :w / :w <path> | save / save-as         |
+----------------+------------------------+
| :q / :q!       | close buffer / discard |
+----------------+------------------------+
| :qa / :qa!     | quit all / force       |
+----------------+------------------------+
| :wq            | save and quit          |
+----------------+------------------------+
| :ls            | list buffers           |
+----------------+------------------------+
| :bn / :bp / :b | navigate buffers       |
+----------------+------------------------+
| :bd / :bd!     | close buffer           |
+----------------+------------------------+

``%`` = current file, ``#`` = alternate file in command args

Appendix — A Taste of More
==========================

This appendix sketches features not covered in the main lessons.

Registers
---------

Prefixing a yank, delete, or paste with ``"<char>`` stores into or
reads from a specific register. Valid register names:

+-----+--------------------------------------------+
| "cy | yank into the system clipboard register  c |
+-----+--------------------------------------------+
| "cp | paste from the system clipboard            |
+-----+--------------------------------------------+

+-----+---------------------------------+
| "5y | yank into in-memory register  5 |
+-----+---------------------------------+
| "5p | paste from register  5          |
+-----+---------------------------------+

(registers 0–9 are symmetric named storage — ``"Ny/"Np`` round-trip)
Note: digit registers are shared with macros — recording ``Q5`` overwrites
any text stored in register 5, and ``"5y`` overwrites any macro there.

+-----+--------------------------------------------------------+
| "kp | paste from the kill-ring head (most recent kill)       |
+-----+--------------------------------------------------------+
| "ky | push yank onto the kill ring (ring only, no clipboard) |
+-----+--------------------------------------------------------+

(older ring entries: cycle with ``[`` and ``]`` after a paste)

+-----+----------------------------------------------------+
| "by | discard the yank (black hole — reads nothing back) |
+-----+----------------------------------------------------+

Macros
------

+---------+--------------------------------------------------------------+
| Q<r>    | start recording into register <r>   (QQ uses the q register) |
+---------+--------------------------------------------------------------+
| Q<r>    | stop recording (same key ends the recording)                 |
+---------+--------------------------------------------------------------+
| q<r>    | replay register <r>                 (qq replays q)           |
+---------+--------------------------------------------------------------+
| <n>q<r> | replay <n> times                                             |
+---------+--------------------------------------------------------------+

HUME uses uppercase to record and lowercase to replay.

Soft Wrap
---------

+--------------------------+------------------------------------------+
| :wrap  :toggle-soft-wrap | toggle line wrapping in the current pane |
+--------------------------+------------------------------------------+

Syntax Highlighting
-------------------

Run ``:plum-install-grammar`` to install the grammar for the current
buffer's language. You can type the whole command, or type a prefix
like ``:plum-i`` and press ``Tab`` to autocomplete it — keep pressing
``Tab`` to cycle through the matches. Press ``Enter`` to run it, then
wait for the grammar to finish installing.

This needs ``git``, ``curl``, the ``tree-sitter`` CLI, and a C compiler on
your system. How you install them depends on your operating system: on
macOS, Homebrew (``brew install git curl tree-sitter``) covers all four,
with a C compiler coming from Xcode's Command Line Tools; on Linux, your
distribution's package manager (``apt``, ``dnf``, ``pacman``, ...) provides
them; on Windows, reach for ``winget``, ``Scoop``, or WSL.

When it's done, this tutor is syntax highlighted: it is a reStructuredText
document, and you just installed the grammar that colours it.

See the user manual to learn more about syntax highlighting and grammars.

Closing Note
------------

.. epigraph::

   Thank you for reading. "What we call a mind is nothing but a heap
   or collection of different perceptions … It is a succession of
   patches … connected by habit rather than necessity."

   — D. Hume (paraphrased)

End of the HUME Tutor. Press ``gg`` to jump back to the beginning,
or ``:bd!`` to close this buffer.
