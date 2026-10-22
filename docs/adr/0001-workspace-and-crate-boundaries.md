# 0001: Workspace and crate boundaries

- Status: Accepted
- Date: 2026-10-22

## Context

Windwatcher needs to support two primary usage modes: embedding its functionality
directly into Rust applications and running the official standalone, self-hosted
server. Those concerns should not force hosting-specific dependencies or behavior
into the reusable library.

## Decision

Use a Cargo workspace initially containing `windwatcher` and `windwatcher-server`.

`windwatcher` is the embeddable library. `windwatcher-server` is the official
standalone application and depends on `windwatcher`; the dependency is
one-directional. The server package produces the `windwatcher` binary.

The two packages are independently versioned.

## Consequences

Positive consequences include:

- The server becomes a real consumer of the public library.
- Hosting-specific concerns can remain outside the reusable core.
- Library consumers are not forced to adopt the server stack.
- The packages may evolve and release independently.

Trade-offs include:

- Multiple packages must be maintained.
- Package-specific changelogs and releases are required.
- Future release automation must account for package dependency and version
  ordering.
