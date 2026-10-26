# Changelog

All notable user-facing changes to this package are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Breaking:** message reads, send/edit/delete results and paginated history now
  return `MessageView`, with sender username and a shallow current reply preview
  loaded through SQLite joins, including disabled identities and tombstones.

- **Breaking:** Direct/Group creation and Group-member addition now depend only
  on `ConversationStore`; unavailable participants are reported semantically.
  Storage atomically rejects disabled/missing new members while preserving
  existing Directs and memberships, including idempotent member additions.

- **Breaking:** get/edit/delete message use cases and `MessageStore` item methods
  now accept globally unique `MessageId` without `ConversationId`. Item outcomes
  no longer include `ConversationNotFound`; parent deletion yields `MessageNotFound`.

### Added

- `GetUser` application lookup by identity ID, independent of authentication and
  authorization and without exposing credentials.
- Semantic `SessionStoreError::OwnerDisabled`/`OwnerNotFound` creation rejections
  and `CreateSessionError::UserNotFound`, with atomic owner validation required
  by the portable adapter contract rather than backend-specific conflict inference.

- Reusable opaque cursor pagination with `PageRequest`, `Page<T>`, and validated
  `PageLimit` (default 50, maximum 100).
- `ListMessages` paginated history using newest-first UUIDv7 keyset ordering,
  versioned scope-bound cursors, current membership checks and tombstones.

- Text message model and exact portable content, with configurable new-write
  `MessagePolicy` (128 KiB default measured in UTF-8 bytes).
- Send, single-message retrieval, lightweight replies, author editing and
  author/Group-moderator deletion with content-free structural tombstones.
- Explicit monotonic membership read markers, derived unread counts excluding
  own/deleted messages, and atomic historical-read initialization on Group join.
- Semantic `MessageStore` port, SQLite message/read-state migration and adapter,
  and portable lifecycle contracts plus SQLite integrity/concurrency tests.

- Direct (including self Direct) and Group conversations, membership roles,
  Group administration and ownership lifecycle use cases.
- SQLite conversation persistence with transactional Direct uniqueness,
  membership, and Group ownership invariants.

- `CreateLocalUser`, a trusted local-user provisioning use case independent of
  self-registration policy.

- Optional SQLx-backed SQLite persistence through `SqliteStorage`, including
  explicit embedded migrations and `IdentityStore`/`SessionStore` implementations.
- SQLite storage contract and concurrency coverage for identity creation and
  refresh-token rotation.
- First-class sessions with absolute expiration, individual revocation, and
  owner-scoped asynchronous session storage contracts.
- Opaque 256-bit refresh tokens, SHA-256 persistence fingerprints, and atomic
  refresh-token rotation.
- Format-independent access-token abstractions and an optional HS256 JWT adapter.
- Session revocation when a user is disabled.
- Local registration and password authentication through asynchronous identity
  storage and password hashing ports, with explicit self-registration policy.
- Zeroized password secrets, opaque PHC hashes, and an Argon2id adapter.
- Persisted enable/disable use cases for trusted operator callers.
- Strongly typed UUIDv7 identifiers for core domain entities.
- Initial user identity model with canonical validated usernames and enable/disable behavior.

### Fixed

- SQLite session creation atomically rejects missing or disabled owners, preventing
  a disable-during-login race from creating a new active session.
