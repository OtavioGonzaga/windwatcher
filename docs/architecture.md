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

The reusable `windwatcher` crate contains UUIDv7 identifiers, the user identity
model, and local identity application services. Registration, authentication,
and persisted enable/disable operations depend on the `IdentityStore` and
`PasswordHasher` ports. The Argon2id adapter implements the password port;
concrete persistence is supplied by the embedding application.

Ports return native Rust `Send` futures and impose no executor. The initial
Argon2id adapter performs CPU-bound work on the polling thread; hosts can wrap
the port with runtime-specific blocking-pool execution. See
[local authentication](authentication.md) and
[ADR 0003](adr/0003-local-credentials-and-authentication.md).
