# Sessions and tokens

## Long-lived realtime connections

An access JWT is a short transport credential; the session is the authority behind
a long-lived [realtime socket](realtime.md). `SessionStore::find_session` and
`ValidateSession` provide transport-independent ownership/lifecycle validation,
collapsing missing, wrong-owner, revoked and expired sessions to an inactive result
while preserving storage failures separately. A session at its expiration is inactive.

The server validates at ticket issuance and again during handshake preparation.
Local logout invalidates tickets and pending/active connections of that session;
other sessions remain connected. Session expiry is a socket deadline. Periodic
revalidation detects external revocation and fails closed on storage failure.
Ordinary HTTP still accepts an already-issued valid JWT until its expiration;
neither that policy nor refresh rotation changes for realtime.

## Session lifecycle

`Session` is first-class application state. A user may hold multiple concurrent
sessions. Each session has a UUIDv7 identifier, an owner, creation time, absolute
expiration, and optional revocation time. Refreshing never extends its expiration.

`CreateSession` accepts an already authenticated, enabled `User`; authentication
and password verification remain separate in `AuthenticateLocalUser`. It returns a
`SessionGrant` containing the session, an access bearer token, and an opaque refresh
bearer token. Embedders provide `SessionStore`, `RefreshTokenProvider`, and
`AccessTokenCodec` implementations.

## Session policy

The default access-token lifetime is 15 minutes and the default session lifetime is
30 days. Both lifetimes must be positive and access-token lifetime cannot exceed the
session lifetime. Because access-token claims have whole-second precision, access
token lifetime must be at least one second. Access-token expiry is capped at the
session's absolute expiry.
All time is supplied explicitly by callers and expiration arithmetic is checked.

## Refresh tokens and rotation

Refresh tokens are 32 random bytes from the operating-system CSPRNG, encoded as
canonical unpadded URL-safe base64. They are bearer secrets and are redacted from
debug output. Storage receives only their SHA-256 fingerprints, not raw values.
SHA-256 is appropriate because refresh tokens have 256 bits of generated entropy;
unlike human-selected passwords, they do not need Argon2id's expensive hardening.

Every successful `RefreshSession` call atomically rotates the persisted fingerprint.
The store must implement compare-and-swap semantics: two concurrent uses of the
same token yield exactly one success. Old tokens are immediately invalid. v1 does
not retain refresh history or perform token-family reuse detection.

## Revocation and user state

`RevokeSession` is owner-scoped and idempotent. `DisableUser` revokes all of the
user's sessions, including when a retry finds the user already disabled. If the
user-state update succeeds but session revocation fails, the user remains disabled;
a retry performs revocation again. `RefreshSession` independently checks current
user state, so disabled or missing users cannot obtain a new access token.

Enabling a user does not restore old sessions. The user must authenticate and create
a new one.

## Access tokens

Access tokens are format-independent through `AccessTokenCodec`; verification always
receives an explicit current time. The optional `jwt-hs256` feature provides the
official HS256 JWT codec. It requires a signing key of at least 32 bytes, exact issuer matching,
and emits only `sub`, `sid`, `iat`, `exp`, and `iss` claims.

Access-token claims use whole-second timestamps, matching the JWT NumericDate
representation. The HS256 adapter rejects oversized encoded JWT inputs before
decoding them; refresh-token parsing likewise rejects values that are not the exact
canonical 43-character representation before base64 decoding.

Access tokens are not persisted, blacklisted, or checked against session storage on
each request. Consequently, revoking a session (including through user disable)
immediately blocks refresh but an access token already issued remains cryptographically
valid until its short expiration. This intentional v1 trade-off bounds the window to
at most the configured access-token lifetime.

## HTTP surface

The server's [HTTP adapter](http-api.md) composes credentials with `CreateSession`
at `POST /v1/sessions` (201), rotates via `POST /v1/sessions/refresh` (200) and
revokes via bearer-authenticated `DELETE /v1/sessions/current` (204 with no body).
Tokens are JSON data, never cookies. Grants are `Cache-Control: no-store` and
transport timestamps are RFC 3339 UTC strings. Protect refresh tokens as secrets.

**HTTP logout does not blacklist an already-issued access token.** It immediately
blocks future refresh, while that access token can still call authenticated
routes until expiration. Repeated deletion, including an absent session, is 204.
`GET /v1/users/me` uses `GetUser`, may return a disabled identity during that
window, and maps a missing identity to an invalid access principal (401).

`SessionStore::create_session` atomically requires a currently enabled persisted
owner, even if the authenticated `User` snapshot predates disabling or deletion.
Every adapter must return `OwnerDisabled` or `OwnerNotFound` according to the
same atomic snapshot that permits insertion, persisting neither session nor
refresh fingerprint on rejection. Genuine persistence/uniqueness conflicts
remain `Conflict`; callers must not reconstruct owner state with a later lookup.
The portable session-store contract covers both owner rejections and genuine
conflicts. SQLite implements the check and insert under one immediate transaction.

`CreateSession` maps these owner rejections to `UserDisabled` and `UserNotFound`.
HTTP handles only these application errors (403 disabled or 401 invalid
credentials); it has no knowledge of SQLite error conventions. A subsequent
re-enable cannot change the already-classified rejection into an internal error.
