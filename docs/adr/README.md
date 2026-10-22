# Architectural Decision Records

Architectural Decision Records (ADRs) capture significant architectural decisions
made by this repository and the context behind them.

Create an ADR when a change establishes or materially changes an architectural
boundary, contract, or long-lived technical direction. Routine implementation
choices and local refactors do not need one.

ADRs use zero-padded sequential numbers and descriptive filenames, such as
`0001-workspace-and-crate-boundaries.md`. Each ADR includes a status, normally
`Proposed`, `Accepted`, or `Superseded`.

ADRs document concrete decisions, not speculative roadmap architecture. An ADR
should normally be introduced in the same change that implements its decision.
When a decision is superseded, preserve the original ADR and mark it accordingly
rather than silently rewriting history.

Use [the template](0000-template.md) as a starting point.
