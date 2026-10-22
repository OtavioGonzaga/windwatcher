# 0003: Local credentials and authentication

- Status: Accepted
- Date: 2026-10-22

## Context

The initial identity domain needs local registration and authentication before
any database, transport, or session implementation exists. Consumers must be
able to embed these use cases without inheriting an application runtime, while
future database adapters need asynchronous I/O. Credentials must not become
part of the messaging-domain user identity.

## Decision

Keep credentials separate from `User`. Use opaque `SecretPassword` and
`PasswordHash` values with redacted debug output and explicit secret/persistence
accessors. Owned plaintext is zeroized on drop. New passwords require eight
Unicode scalar values and all input is bounded to 1024 UTF-8 bytes. Neither
normalization nor composition requirements apply. Authentication checks only
the input bound, allowing verification of older password policies.

Application services depend on `IdentityStore` and `PasswordHasher`. These ports
use native return-position `impl Future + Send` with `Sync` implementations,
and static dispatch. They require neither `async-trait` nor a particular runtime.
Storage operations are defined by current use cases: atomic user-plus-credential
creation with canonical username uniqueness, local authentication lookup, user
lookup, and narrow state updates. There is no concrete production store yet.

The official password adapter generates Argon2id v19 PHC hashes with 19456 KiB,
two passes, one lane, 32-byte output, and fresh OS-random 16-byte salts.
Verification honors encoded parameters, rather than requiring the current
generation profile. No pepper or automatic rehash policy is introduced.

Self-registration requires an explicit open/closed policy. Authentication
performs dummy cryptographic verification for invalid/missing usernames and
checks disabled state only after a correct password. Operational failures have
a generic authentication error with diagnostic sources. Enable/disable services
are trusted operator capabilities and skip no-op writes; they introduce no roles.

## Consequences

- Embedders can implement asynchronous persistence and replace password hashing
  independently, while transports and sessions remain separate concerns.
- Storage adapters must enforce atomic creation and uniqueness under concurrency;
  application preflight checks cannot satisfy the contract.
- Generic services use static dispatch; these native future-returning ports are
  not directly usable as trait objects.
- The initial Argon2 adapter blocks the polling thread. Runtime-aware hosts must
  arrange blocking-pool execution and concurrency limits outside the core.
- PHC parsing checks completeness and syntax; adapter-specific validation occurs
  on verification. Persisted parameters must come from trusted credential storage.
- Dummy verification reduces trivial username timing differences but cannot
  guarantee constant end-to-end latency across storage or historical cost profiles.
- State updates are last-write-wins, without optimistic concurrency control.
