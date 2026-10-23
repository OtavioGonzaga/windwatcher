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

The server is the application composition root: CLI parsing, runtime paths,
configuration, secret resolution, tracing, adapter selection, and operator
workflows belong there rather than in the reusable crate.

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
model, local identity application services, and session lifecycle use cases.
Registration, authentication, and user state depend on `IdentityStore` and
`PasswordHasher`; session creation, refresh, and revocation depend on
`SessionStore`, `RefreshTokenProvider`, and `AccessTokenCodec`. The optional
HS256 JWT adapter implements the access-token port, but JWT is not required by
embedding applications. The optional `sqlite` feature provides the first
official persistence adapter: `SqliteStorage` implements both storage ports
while keeping SQLx and its Tokio runtime requirement at the infrastructure
boundary.

Ports return native Rust `Send` futures and impose no executor. The initial
Argon2id adapter performs CPU-bound work on the polling thread; hosts can wrap
the port with runtime-specific blocking-pool execution. See
[local authentication](authentication.md) and
[ADR 0003](adr/0003-local-credentials-and-authentication.md).
