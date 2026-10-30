# Runaway-Script Protection: The Watchdog Timer

## The problem

HUME embeds a Scheme interpreter, Steel, so plugins can bind keys, change
settings and define commands. Scripts run on the same thread as the event
loop, because that is the only safe way to let them read and write editor
state without locks. That creates an obvious hazard: if a plugin's script
enters an infinite loop, or a command waits for something that never
arrives, the event loop is blocked and the editor freezes.

A second hazard is a script that does part of its work and then hits an
error. HUME fails fast: the evaluation stops at the error, and the error
appears in `:messages`. What the script had already done depends on where it
ran:

- At the top level of `init.scm`, changes made before the error stay
  (options set, commands defined, hooks registered), while deferred changes
  from that run, such as key bindings, are discarded.
- In a plugin body, the commands, hooks, key bindings and other deferred
  changes it made are rolled back, so the failed plugin leaves none of them
  behind. Options it set before the error stay set.

`:reload-config` rebuilds everything from scratch, and is the way to recover
from state a failed run left behind.

## The watchdog timer

HUME keeps one watchdog thread alive for as long as the scripting engine
exists, instead of starting a thread per evaluation: most evaluations are
commands that return in microseconds, and a thread per evaluation would be
wasted work on that path. The thread is *armed* before each evaluation with
a time budget:

- 10 seconds for `init.scm`, and for a plugin that a hook, a language or one
  of its own commands loads;
- 1 second for a command, a hook handler or a callback, and for a plugin a
  command loads through `call!`, which runs inside that command's budget.

Both budgets are settings.

If the evaluation returns in time, the watchdog is told to stand down. That
call waits for the watchdog to acknowledge it, so a timer that is about to
fire cannot leak its interrupt into the next evaluation.

If the budget runs out first, the watchdog interrupts the script. Steel
checks before every instruction whether it has been told to stop, and the
watchdog can set that flag from its own thread. From then on every
instruction raises an error, so the script stops wherever it is, even in a
loop that never calls into the editor. The flag stays set until the
evaluation is over: an error handler in the script cannot catch the
interrupt and carry on, but none of the script's own clean-up handlers run
either. HUME clears the flag before the next evaluation starts.

A plugin that was loading when its budget ran out is left half-run, and its
own error handling never gets the chance to mark it failed. The end of the
evaluation does it instead: every plugin load still open is marked failed and
rolled back, as if its body had raised an error.

## What the budget counts

The budget measures time spent *running Scheme*, not wall-clock time. A
plugin that clones a repository, downloads a language server or deletes a
large directory spends most of its time waiting on the operating system, and
that waiting is not the script misbehaving. HUME's own calls that wait this
way (running a program and waiting for it, deleting a directory tree, opening
a grammar's library) tell the watchdog when the wait starts and ends, and the
watchdog stops its clock in between. A one-second command budget therefore
stops a runaway loop after one second of looping, but lets an install that
waits two minutes on `git` finish.

Waiting that HUME does not do on the script's behalf still counts. A script
that sleeps chose to hold the editor, so its sleep counts, and so does
waiting in Steel's own process functions.

## Waiting without polling

While armed, the watchdog thread waits for a message rather than sleeping a
fixed time, and checks the deadline every time it wakes instead of treating
the wake itself as the signal to fire. An early or spurious wake can never
trip the interrupt before the budget has run out, and standing the watchdog
down is immediate instead of waiting out the rest of the budget.

---

*See also: [Plugin Architecture](plugin-architecture.md) for how plugin
bodies load; [Plugin Attribution](plugin-attribution.md) for what a failed
plugin had registered.*
