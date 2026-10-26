# HTTP API

## Availability and versioning

The server package provides a constructable, tested HTTP router. The CLI does
**not** serve it yet: listener binding, graceful shutdown, health/readiness and
the process lifecycle are intentionally deferred. No `serve` placeholder exists.

Public resources live below `/v1`. Documentation lives at `/docs` (redirecting to
`/docs/`) and `/openapi/v1.json`. Future API majors can coexist as `/v2` and
`/openapi/v2.json`; there is no mutable "latest" specification alias.

## Resource model

The current surface is exactly:

| Method | Resource | Purpose |
| --- | --- | --- |
| POST | `/v1/users` | Public registration, 201 without a session |
| POST | `/v1/sessions` | Authenticate credentials and persist a session, 201 |
| POST | `/v1/sessions/refresh` | Rotate a refresh token, 200 |
| DELETE | `/v1/sessions/current` | Revoke the current session's refresh capability, 204 |
| GET | `/v1/users/me` | Retrieve the authenticated identity, 200 |

Registration remains a registered route when closed and returns
`403 registration.closed`. It reuses `auth.registration`, not an HTTP-specific
toggle. No arbitrary user lookup, session listing, messaging or health endpoint
is implemented. Axum also supplies the normal implicit HEAD handling for GET.

## Authentication

Tokens are JSON transport data, not cookies. Access tokens are submitted only as
`Authorization: Bearer <access-token>`. Query parameters, cookies, request bodies
and alternative headers are not access-token sources. Multiple Authorization
values are rejected. Bearer scheme matching is case-insensitive.

Missing, invalid, expired and otherwise rejected access credentials produce
`401 auth.invalid_access_token`. Operational verification failures produce a
generic 500. The extractor verifies the token's signature and claims without
consulting identity or session storage. All authentication 401 responses include
`WWW-Authenticate: Bearer`, without credential/error internals.

Unknown usernames, invalid username syntax, wrong passwords and oversized login
passwords all produce `401 auth.invalid_credentials`. Correct credentials for a
disabled user produce `403 auth.user_disabled`. Persistence atomically prevents
an intervening disable from creating a new active session.

This is Windwatcher's local authentication/session protocol, not OAuth or OIDC.
CORS permissions do not authorize a caller.

## Session lifecycle

Login and refresh return the same `SessionGrantResponse`, including access and
refresh tokens, `token_type: "Bearer"`, session ID and expiration timestamps.
These responses carry `Cache-Control: no-store`. Treat refresh tokens as bearer
secrets and protect them in client storage; never include them in logs or URLs.

Submit a refresh token as JSON to `/v1/sessions/refresh`, without requiring an
access token. Successful rotation immediately invalidates the old refresh token.
Malformed, unknown, stale, expired, revoked and unusable-user refresh attempts all
produce `401 auth.invalid_refresh_token`. Concurrent rotation has one winner.

**Logout prevents future refresh; it does not blacklist an already-issued access
token.** Repeated logout succeeds with 204, including when the session is already
absent. There is no response body. Existing access tokens remain usable until
expiration, including after user disable. `/users/me` can consequently return a
disabled user during that bounded interval. A missing identity for a valid JWT
subject instead produces `401 auth.invalid_access_token`.

See [sessions](sessions.md) for the authoritative reusable lifecycle semantics.

## Request and response DTOs

DTOs are server-owned representations, not serialized domain entities. User
responses contain only identity and state timestamps, never credentials. Small
command DTOs reject unknown fields and require their documented JSON types.
Secret-bearing DTOs have no unredacted Debug implementation and their owned
secret fields are zeroized on drop. Password work uses a bounded Tokio blocking
adapter around the existing Argon2 implementation, including dummy verification;
request cancellation does not release a still-running worker's concurrency slot.

All response timestamps are RFC 3339 UTC strings, described as OpenAPI
`string/date-time`. Domain and SQLite timestamp encodings are not exposed.
User IDs, session IDs and problem request IDs are described as `string/uuid`;
OpenAPI does not duplicate UUIDv7 domain validation with a regular expression.

## Error contract and validation semantics

Errors use RFC 9457 with `Content-Type: application/problem+json` and fields
`type`, `title`, `status`, `detail`, `code`, and `request_id`. Problem types are
stable `urn:windwatcher:problem:...` identifiers. Clients must branch on `code`,
not parse human-readable titles/details. `status` matches the HTTP status.
No request URI, query string or internal error source is echoed in problems.

| Status | Meaning / codes |
| --- | --- |
| 400 | Malformed syntax/shape, missing/wrong/unknown fields: `request.invalid_json` |
| 401 | `auth.invalid_credentials`, `auth.invalid_access_token`, `auth.invalid_refresh_token` |
| 403 | `registration.closed`, `auth.user_disabled` |
| 404 | Unknown API route: `request.not_found` |
| 405 | Unsupported method: `request.method_not_allowed`, preserving `Allow` |
| 409 | `user.username_taken` |
| 413 | JSON body exceeds 2 MiB: `request.payload_too_large` |
| 415 | Missing or unsupported JSON Content-Type: `request.unsupported_media_type` |
| 422 | Structurally valid but semantically rejected: `user.invalid_username`, `user.invalid_password` |
| 429 | `rate_limit.exceeded`, with `Retry-After` in seconds |
| 500 | `internal.error`, with generic detail and server-side diagnostics |

Transport validates structure; the domain/application owns username and password
semantics. No duplicate regex or password policy exists in DTO validation.
Middleware can reject a request before its body is parsed. OpenAPI documents
operation-specific supported statuses.

## Request IDs and tracing

Every request receives a server-generated UUIDv7 `X-Request-Id`, replacing any
client-supplied value. The response header, Problem Details `request_id` and
tracing span use the same value, including 404, 405, 429 and documentation requests.
Spans include method, matched route, status and latency, not full query-bearing
URIs. Authorization/Cookie/Set-Cookie are marked sensitive. HTTP tracing never
logs request/response bodies or bearer values. Successful lifecycle events use
user/session IDs inside the correlated request span.

## OpenAPI and Swagger

OpenAPI 3.1 is generated from the same Utoipa-aware composition that registers
runtime routes. Operation IDs and schema names are intentional public contracts.
`Users` and `Sessions` tags organize the operations; `bearerAuth` is required only
for current-user retrieval and current-session deletion.

Response headers are part of the generated contract: every declared response
includes `X-Request-Id` (UUID), every 401 includes `WWW-Authenticate: Bearer`,
every 429 includes positive integer `Retry-After` seconds, and successful login
and refresh responses declare `Cache-Control: no-store`. Common headers are
applied centrally to the generated responses; grant cache policy remains next
to its response annotation, without maintaining a separate route registry.

Swagger loads `/openapi/v1.json`. UI assets are vendored and minified: normal
builds do not download Swagger assets, and the UI does not require a CDN.
Documentation endpoints are public, absent from the domain API specification and
outside API CORS/rate-limit policy. No generated spec is committed as a second
source of truth.

## CORS

The `/v1` policy denies cross-origin browser access by default. Configure explicit
origins or `*` alone; credentials are never enabled. Authorization and Content-Type
request headers and GET/POST/DELETE methods are permitted. Valid preflights are
handled before authentication and quotas. Allowed-origin API errors retain CORS
headers, and browsers can read `X-Request-Id` and `Retry-After`.

## Rate limiting

Governor token buckets are in-memory, per process and per immediate peer IP.
The general quota covers API requests, with an additional shared stricter quota
for POST registration/login/refresh. Current-user and logout use only the general
quota. Documentation assets spend neither quota. Inactive keys are cleaned
opportunistically every 1024 API quota checks; no background worker is started.

Forwarded, X-Forwarded-For, X-Real-IP and vendor IP headers are ignored. Behind
a reverse proxy, the limiter sees the proxy as peer: adjust/disable quotas until
explicit trusted-proxy support is delivered. This is not a distributed limiter.

The future serving runtime must use Axum's connect-info make-service, supplying
`ConnectInfo<SocketAddr>`. A missing peer with limiting enabled is an internal
configuration error (500), never an unlimited request or a forwarded-IP fallback.

See [server configuration](server-configuration.md) for defaults and overrides.

## Current limitations

No listener, health/readiness, cookies, CSRF mode, OAuth/OIDC, trusted proxies,
distributed quotas, instant JWT revocation or Idempotency-Key replay handling is
implemented. These are not advertised as supported protocol capabilities.
