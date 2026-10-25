# Runaway-Script Protection: The Watchdog Timer

## The problem

HUME embeds a scripting engine — Steel, a Scheme dialect — so plugins can
bind keys, change settings, and define new commands. Scripts run on the same
thread as the event loop, because that's the only safe way to let them read
and write editor state without locks. But that creates an obvious hazard: if a
plugin's init script enters an infinite loop, or a command handler hangs
waiting for something that never arrives, the event loop is blocked and the
editor is frozen.

There's a second hazard: a plugin that partially completes before hitting an
error. HUME handles this with **fail-fast** semantics: the eval aborts as soon
as an error occurs and the error surfaces in `:messages`. Partial changes made
before the error are not rolled back when they happen at the top level of
`init.scm` — options set, commands defined, and hooks registered all survive,
though deferred changes such as key bindings from the failed run are
discarded. A *plugin body* that fails mid-evaluation is one step more
careful: any commands, hooks, key bindings, and other pending changes it
already made before the error are rolled back, so a half-loaded plugin
leaves no footprint. Re-running `:reload-config`
rebuilds everything from scratch either way and is the recovery path for
state left behind at the top level.

## The watchdog timer

HUME keeps one watchdog thread alive for the lifetime of the scripting
engine, rather than spawning a fresh thread for every evaluation — spawning
per eval would be wasted work on the hot path, since most evaluations are
commands that return in microseconds. Instead, the thread is *armed* before
every script evaluation — during startup (loading `init.scm` and plugins), on
every individual command invocation, and before every lifecycle hook fires —
with a configurable budget: 10 seconds for init/plugin loads, 1 second for
command calls and hook handlers.

If the eval returns within the budget, the watchdog is told to stand down
before it can fire — and the standing-down call waits for the watchdog's
acknowledgement, so a timer that is already mid-expiry cannot leak its
interrupt into the *next* evaluation's budget.

If the budget expires first, the watchdog interrupts the script itself. Steel
checks, before every instruction it executes, whether the thread running it
has been told to stop, and the watchdog thread can set that state from
outside. From then on every instruction raises an error, so the script stops
wherever it is, even in a loop that never calls into the editor. The state is
*sticky*: it stays set until the evaluation is over, so an error handler in
the script cannot catch the interrupt and carry on, but it also means none of
the script's own clean-up handlers run. HUME clears it before the next
evaluation starts.

A plugin that was loading when its budget ran out is left half-run, and its
own error handling never gets the chance to mark it failed. The evaluation's
end does it instead: every plugin load still open is marked failed and its
commands and hooks are removed, the same as when its body raises an error.

## What the budget counts

The budget measures time spent *running Scheme*, not wall-clock time. A
plugin that clones a git repository, downloads a language server, or deletes
a large directory spends most of its time waiting on the operating system,
and none of that is the script misbehaving. The calls that wait this way
(running a program and waiting for it, compiling a grammar, deleting a
directory tree) tell the watchdog when a
wait starts and ends; the watchdog stops its clock in between and resumes it
with whatever budget was left. A one-second command budget therefore stops a
runaway loop after one second of looping, but lets an install that waits two
minutes on `git` finish. A script that sleeps is different: it chose to hold
the editor, so its sleep counts.

## Waiting without polling

While armed, the watchdog thread sleeps in a wake-on-message wait rather than
a plain sleep, and re-checks the deadline every time it wakes rather than
trusting the wake itself as a signal to fire — an early or spurious wake can
never trip the interrupt before the budget has actually elapsed. This also
means standing the watchdog down is fast: no waiting out the remainder of the
budget on the common case where the eval finishes early.
