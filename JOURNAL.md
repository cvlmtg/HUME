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
