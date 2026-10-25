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
model, conversation and membership application services, local identity
application services, session lifecycle use cases, and core text-message and
read-state use cases.
Registration, authentication, and user state depend on `IdentityStore` and
`PasswordHasher`; session creation, refresh, and revocation depend on
`SessionStore`, `RefreshTokenProvider`, and `AccessTokenCodec`. The optional
HS256 JWT adapter implements the access-token port, but JWT is not required by
embedding applications. The optional `sqlite` feature provides the first
official persistence adapter: `SqliteStorage` implements identity, session, and
conversation and message storage ports
while keeping SQLx and its Tokio runtime requirement at the infrastructure
boundary.

Message application operations depend on `MessageStore`, implemented by the
optional SQLite adapter:

```text
Message use cases → MessageStore → SQLite adapter
```

`MessageContent` defines exact portable text (only NUL is rejected), while
`MessagePolicy` controls UTF-8 byte acceptance for new sends/edits. `Message`
defines active/tombstone structural state independently of current size policy.
The application generates UUIDv7 ordering keys; persistence atomically enforces
membership, reply availability, edit/delete permissions and monotonic read
markers. Membership owns read state and unread counts are derived. The server
resolves its message configuration to the reusable library policy and delegates
the message port through concrete storage enum dispatch. See
[messages](messages.md) and [ADR 0008](adr/0008-message-lifecycle-and-read-state.md).
No HTTP, pagination, realtime, events or workers are implemented by this delivery.

Ports return native Rust `Send` futures and impose no executor. The initial
Argon2id adapter performs CPU-bound work on the polling thread; hosts can wrap
the port with runtime-specific blocking-pool execution. See
[local authentication](authentication.md) and
[ADR 0003](adr/0003-local-credentials-and-authentication.md).
