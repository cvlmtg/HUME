# Runaway-Script Protection: Watchdog and Eval Rollback

## The problem

HUME embeds a scripting engine — Steel, a Scheme dialect — so plugins can
bind keys, change settings, and define new commands. Scripts run on the same
thread as the event loop, because that's the only safe way to let them read
and write editor state without locks. But that creates an obvious hazard: if a
plugin's init script enters an infinite loop, or a command handler hangs
waiting for something that never arrives, the event loop is blocked and the
editor is frozen.

There's a second hazard: a plugin that partially completes before hitting an
error. Suppose a plugin binds 20 keys, registers 5 commands, then crashes on
the 21st key binding. Should HUME apply the first 20 bindings and then stop?
Or should the error mean the plugin never loaded at all? Partial application
is hard to reason about — from the user's perspective, the plugin either
worked or it didn't.

## The watchdog timer

HUME arms a timer thread before every script evaluation — both during startup
(loading `init.scm` and plugins) and on every individual command invocation.
The timer is set to a configurable budget: 10 seconds for init/plugin loads,
1 second for command calls.

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
out the remainder of the budget on the fast path.

## Eval snapshot and rollback

Every plugin load and every Steel eval that can modify editor state is wrapped
in a *snapshot*. Before the eval starts, the current state of the keymap,
settings, plugin ledger, and hook registry is captured. After the eval returns:

- **Success**: the snapshot is discarded. The state changes made during the
  eval are the intended result.
- **Failure (error or watchdog timeout)**: the snapshot is *restored*, reverting
  every change the eval made — bindings, settings, registered commands, all of
  it. The editor is back to the state it was in before the problematic plugin
  ran.

The restore is all-or-nothing: there is no "partial rollback" or "keep the
bindings that succeeded". Either the whole plugin loaded cleanly, or none of
it did. This makes plugin failures predictable from the user's perspective.

One deliberate exception: error messages accumulated during the failed eval are
**not** rolled back. The user needs to see what went wrong — wiping the
messages along with the state changes would make silent failures invisible.

## Why this matters for plugin ordering

The rollback means plugins are isolated from each other's errors. If three
plugins are loaded in sequence and the second one fails, the first one's
changes remain (its eval completed successfully before the snapshot for the
second was taken), the second one's changes are rolled back, and the third one
runs against the same state as if the second had never been attempted.

Together with the [plugin ledger](plugin-ledger.md), which tracks who owns what
after successful loads, this gives HUME a two-level safety net: rollback on
load failure, and clean unload for successfully loaded plugins that are later
removed.
