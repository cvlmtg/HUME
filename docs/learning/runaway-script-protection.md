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
`init.scm` — keys bound, options set, hooks registered all survive. A *plugin
body* that fails mid-evaluation is one step more careful: any commands it
already registered before the error are rolled back, so a half-loaded plugin
does not leave orphan commands in the registry. Re-running `:reload-config`
rebuilds everything from scratch either way and is the recovery path for
state left behind at the top level.

## The watchdog timer

HUME arms a timer thread before every script evaluation — both during startup
(loading `init.scm` and plugins), on every individual command invocation, and
before every lifecycle hook fires. The timer is set to a configurable budget:
10 seconds for init/plugin loads, 1 second for command calls and hook
handlers.

If the eval returns within the budget, the timer is cancelled immediately and
everything proceeds normally. If the budget expires before the eval returns,
the timer sets a shared flag to `true`.

Scripts cooperate by calling `(hume/yield!)` at their yield points — the Steel
equivalent of "check if I should stop". Each `(hume/yield!)` call reads the
flag. If it's set, the call aborts the script with an error.

The cancellation is *cooperative*: HUME cannot forcibly stop a Steel program
mid-instruction the way an OS can terminate a process. A script that never
calls `(hume/yield!)` will still run to completion even after the budget
expires — the flag is only an interrupt request, not a hard kill. For well-
behaved plugins this is invisible; for misbehaving or long-running ones it
bounds the freeze to the interval between yield points.

The watchdog thread itself uses a sleep-with-wake mechanism rather than a plain
sleep: when the eval returns and the watchdog is cancelled, a wakeup call
immediately resumes the watchdog thread so it can exit — there's no sleeping
out the remainder of the budget on the fast path. The watchdog also resets
the interrupt flag after every eval, so a watchdog trip on one eval cannot
bleed into the next. The mechanism is designed so that a future Ctrl-C handler
can set the same flag from outside the script and abort long-running code by
the same cooperative protocol — for now that wiring is not yet hooked up.
