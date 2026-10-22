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

This repository currently contains only the project foundation. Domain modules,
adapters, runtime functionality, and their ADRs will be introduced incrementally
with the deliveries that implement them.
