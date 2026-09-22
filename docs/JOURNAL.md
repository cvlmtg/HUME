## 13/03/2026 Basic edit operations

Basic edit operations seem simple: you just need to add or delete some text.

Then Claude tells you it’s going to implement a *right-to-left* approach for X and Y reasons. You approve the plan, only for it to then tell you it implemented the *left-to-right* approach instead.

Naturally, you try to understand which approach is actually better or faster. You hit your usage limits and pivot to Gemini, who insists that you really should be using changesets. You’re still trying to get a straight answer on L2R vs. R2L, and Gemini is just like: "Sure, this one is better, but you really should use changesets! *Seriously!*"

So it's back to Claude. Implement changesets, update the docs, update the plan. Then you actually check the code: do we really need to convert everything to strings just to apply a changeset? do we really need to clone the buffer before modifying it?

Agentic programming can be fun, but you should never blindly trust your AI agent. The lesson here: if you have a doubt or something seems off, grill your agent with questions until you’re actually satisfied.

## 19/03/2026 Asking questions

You can tell the agent to analyze your goals and ask questions if something is unclear. It works, it's nice, but most of the time the UI isn't the best. I mean, a moment ago copilot-cli told me that the toolbar gets the `isVisible` prop, but then when the user sel...
"Sel.." what? If the question is truncated how can I answer?

So from now on I'll explicitly ask to "briefly describe any unclear or conflicting instructions, or any issue that comes up", so I can update my goals accordingly.
*Hopefully this should avo---*

## 09/04/2026 Stick to the plan

I've noticed that Claude is very fixated on the "blast radius" of its edits. I agree that keeping the blast radius small is a good choice most of the time. But "most" doesn't mean "always, at all costs".

So it comes the day you discuss quite a big refactor: you iron out all the little problems, you challenge and discuss all the proposed ideas and corner cases, etc. Finally you converge on a good plan, you approve it, the implementation starts, and the first thing Claude writes is: "Oh, I have to modify 52 functions and 241 tests — let's take a different approach!"

And here you are, bashing on the keyboard: "NO Claude! Bad Claude! *Bad!* Stick to the plan! *Stick to the plan!*"

The lesson here: if you've already done the work of planning a big refactor, don't let the agent second-guess the plan at implementation time. Be explicit that the scope has been agreed upon and should be followed as-is.

## 16/04/2026 It's a trap!

You know, you start small: you add a new feature, you refactor a bit... Nothing new. Even Opus suggests this approach. But when you know you will need or want a certain big feature, and you feel you should design the current implementation taking that big feature into consideration, well... you're right. Perfectly right. You *should* take it into consideration.

So when Claude tells you that you don't need that, that it will be an easy addition, no expensive retrofit, etc... don't believe it. *It's a trap!* (Insert the usual Star Wars meme here.)

If things go awry, you might want to restart from scratch. Re-prompt Claude with the things you learned along the way — a clear description of the whole picture will (hopefully) lead to a better implementation plan.

## 22/09/2026 The real mistake

A serious entry for once: today I think I learned something valuable. It isn't just about "how to design a piece of software," but rather "how to design a piece of software with an AI". The line is probably a bit blurry here, but since this is a project about agentic programming, I'll focus on the latter.
Here is the context: after the third redesign of the completion system, I finally asked the right question: What are we doing wrong?
The answer might seem simple:

*Every rewrite generalized the framework for multiple sources, and every rewrite has only ever been fed by one: core:lsp. "Sources are first-class", per-invocation snapshots, a pluggable registry, multi-source ranking — all built and re-built for a plurality that, in production, has never existed.*

What does that mean? You cannot build a generalized system and ask the AI to review and validate it without real, concrete usage. When everything remains just theory, there is no way to truly test if it works. As a result, every `/code-review` or `/simplify` run will always find some hypothetical hole or bug that cannot be verified.

The fix has to land before the plan is approved, not after the code exists, however elegant the architecture looks. Words like "first-class", "pluggable", "per-invocation" in a plan are the tripwire — reject at reading time, not in `/code-review`, because by then the code exists and killing it feels like a loss.

If `/simplify` flags a bug with no second source in sight: does it break `core:lsp` today? If not, it's not a bug — it's a bill for a future that hasn't shown up.
