# Architecture

## Project model

Windwatcher is library-first:

```text
windwatcher
→ reusable and embeddable library

windwatcher-server
→ official standalone application built on the library
```

## Dependency direction

```text
windwatcher-server
        ↓
   windwatcher
```

The library must not depend on server-specific concerns.

## Architectural principles

- Design library-first.
- Use Ports and Adapters as the architectural direction.
- Keep application and domain behavior independent of HTTP.
- Keep infrastructure adapters replaceable.
- Make public Rust APIs deliberate and avoid exposing implementation internals by
  default.
- Point dependencies inward rather than forcing the library to depend on hosting
  concerns.
- Avoid speculative abstractions.

## Current state

The reusable `windwatcher` crate now contains the initial core domain primitives:
UUIDv7 identifiers and the user identity model. They remain independent of
infrastructure, persistence, HTTP, and runtime concerns. Adapters and further
domain behavior will be introduced incrementally with the deliveries that
implement them.
