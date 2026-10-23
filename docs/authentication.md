# Local credentials and authentication

All local identity behavior lives in the reusable `windwatcher` library.
`RegisterLocalUser`, `CreateLocalUser`, `AuthenticateLocalUser`, `DisableUser`, and `EnableUser`
are application services wired to caller-provided ports. Authentication returns
only `User`; it does not establish a session or issue tokens. Hosts compose a
successful `AuthenticateLocalUser` call with `CreateSession` when they need a
login/session flow; see [sessions and tokens](sessions.md).

## Passwords

Construct `SecretPassword` from an owned `String`. Construction enforces the
1024-byte UTF-8 input bound, including for authentication, and zeroizes the owned
allocation on failure or drop. Previously made copies and borrowed exposures
cannot be erased by this type. Passwords are never trimmed, case-folded, or
Unicode-normalized. `validate_new_password` additionally requires eight Unicode
scalar values; registration calls it, authentication does not. There are no
composition rules. Oversized input fails before any storage or hashing work.

The types intentionally have no serialization or `Display` implementation.
Debug output redacts both secrets and encoded hashes. Adapters use the explicit
`expose_secret` and `as_encoded` methods and must not log their contents.
`PasswordHash::parse` reconstructs complete PHC values from persistence, rejecting
malformed or incomplete encodings. Algorithm-specific support and parameters are
validated during verification; parse success alone does not imply support by a
particular hasher.

## Port wiring

```rust
use time::OffsetDateTime;
use windwatcher::{
    Argon2idPasswordHasher, IdentityStore, RegisterLocalUser,
    RegisterLocalUserError, RegistrationPolicy, SecretPassword, User,
};

async fn register(
    store: &impl IdentityStore,
    password: &SecretPassword,
    now: OffsetDateTime,
) -> Result<User, RegisterLocalUserError> {
    RegisterLocalUser::new(store, &Argon2idPasswordHasher, RegistrationPolicy::Open)
        .execute("Alice", password, now)
        .await
}
```

Registration policy has no default. `Closed` rejects self-registration before
username validation, hashing, or storage. It does not prohibit future trusted
operator creation. `Open` validates input, creates a UUIDv7 identity with the
supplied timestamp, hashes the exact secret, and invokes atomic storage creation.
Username conflicts map to `UsernameTaken`; operational errors remain distinct.

`CreateLocalUser` is a separate trusted operator provisioning use case. It shares
the same validation, hashing, and atomic persistence behavior, but deliberately
does not inspect `RegistrationPolicy` and never creates a session.

## Storage adapter obligations

`IdentityStore` implements these logical operations:

- `create_local_user`: user and local credential commit together or neither
  commits. Canonical usernames are globally unique even under concurrent calls.
  Enforce this atomically in storage, not through an existence preflight.
- `find_local_auth_by_username`: one logical lookup yielding `User` plus
  `PasswordHash`, or `None` for a missing username/local credential.
- `find_user`: user-only lookup, returning `None` when absent.
- `update_user`: update existing enabled/disabled state and its modification
  timestamp only. Preserve identity, username, creation time and credentials;
  report `NotFound` if the user disappeared. Concurrent changes are last-write-wins.

Normalize failures as `UsernameConflict`, `NotFound`, `Unavailable`, or
`Unexpected`. Sources may carry diagnostics, but must never include credentials.
The in-memory implementation is test-only, not a production feature.

## Verification and operational failures

Authentication canonicalizes the username. Invalid syntax skips storage but
still invokes `PasswordHasher::verify` with `None`. Missing credentials likewise
invoke this dummy path, which must perform comparable cryptographic work and
always return false. Wrong passwords, missing usernames, and invalid usernames
all yield `InvalidCredentials`. A disabled user yields `UserDisabled` only when
their password is correct. Hashing and storage failures yield the same `Internal`
authentication variant and display text; sources retain diagnostic context.

The official adapter uses Argon2id v19, 19456 KiB memory, two passes, one lane,
32-byte output, and a fresh cryptographically random salt for each new hash.
Persisted algorithm parameters are used for verification, including older cost
profiles. Malformed/unsupported parameters are operational errors, not mismatches.
There is no pepper, password mutation, or rehash-on-login behavior.

Dummy verification uses the generation cost profile. This reduces trivial timing
disclosures, not all timing differences from I/O or historical hash costs.
Credential storage is a trusted boundary: do not accept arbitrary caller-supplied
PHC parameters as authentication inputs.

## Hosting and trusted operations

The ports return `Send` futures and do not require Tokio or another executor.
Argon2 is CPU/memory-bound and the initial adapter executes on the polling thread.
A runtime-aware wrapper can own the submitted input and offload work to its
blocking pool. The host is responsible for concurrency limits and scheduling.

`DisableUser` and `EnableUser` take a user ID and explicit timestamp. Missing
users produce `UserNotFound`, including deletion between read and update. A no-op
preserves timestamps and avoids writes. Callers must establish operator trust;
these capabilities do not implement end-user authorization or global admin roles.
