# 0004: Session and token lifecycle

- Status: Accepted
- Date: 2026-10-22

## Context

The library needs revocable long-lived login state for multiple devices and
short-lived bearer access, without adopting a database, runtime, HTTP layer, or JWT
as a mandatory dependency for all embedders.

## Decision

Sessions are first-class state with a 30-day default absolute lifetime and support
multiple sessions per user. Access tokens default to 15 minutes and cannot outlive
their session. An `AccessTokenCodec` port represents their format; the optional
official adapter issues HS256 JWTs with only `sub`, `sid`, `iat`, `exp`, and `iss`.

Refresh tokens are opaque 256-bit OS-random values. Only their SHA-256 fingerprints
are persisted. Every refresh atomically compare-and-swaps its fingerprint. Sessions
can be individually revoked, and disabling a user revokes all sessions. There is no
token-family reuse detection, access-token persistence, or access-token blacklist.

## Consequences

Password, session, and token concerns remain separate and embedders can choose a
non-JWT access-token format. Persisted refresh hashes cannot directly authenticate,
and concurrent refresh has one deterministic winner. Storage adapters must provide
atomic creation and compare-and-swap rotation.

Access revocation is not instantaneous: previously issued access tokens remain valid
until expiry. HS256 verification requires each verifier to know the signing secret.
