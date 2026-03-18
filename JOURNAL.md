## 13/03/2026 Basic edit operations

Basic edit operations seem simple: you just need to add or delete some text.

Then Claude tells you it’s going to implement a *right-to-left* approach for X and Y reasons. You approve the plan, only for it to then tell you it implemented the *left-to-right* approach instead.

Naturally, you try to understand which approach is actually better or faster. You hit your usage limits and pivot to Gemini, who insists that you really should be using changesets. You’re still trying to get a straight answer on L2R vs. R2L, and Gemini is just like: "Sure, this one is better, but you really should use changesets! *Seriously!*"

So it's back to Claude. Implement changesets, update the docs, update the plan. Then you actually check the code: do we really need to convert everything to strings just to apply a changeset? do we really need to clone the buffer before modifying it?

Agentic programming can be fun, but you should never blindly trust your AI agent. The lesson here: if you have a doubt or something seems off, grill your agent with questions until you’re actually satisfied.
