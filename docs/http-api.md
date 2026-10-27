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
| GET | `/v1/users?username=alice` | Exact enabled-user discovery |
| GET / POST | `/v1/conversations` | Paginated inbox / open Direct or create Group |
| GET / PATCH | `/v1/conversations/{conversation_id}` | Consumer-ready detail / rename Group |
| GET | `/v1/conversations/{conversation_id}/members` | Paginated public membership attribution |
| PUT / PATCH / DELETE | `/v1/conversations/{conversation_id}/members/{user_id}` | Add / change role / remove Group member |
| PUT | `/v1/conversations/{conversation_id}/owner` | Transfer Group ownership |
| DELETE | `/v1/conversations/{conversation_id}/members/me` | Leave Group; delete owner-only Group |
| GET / POST | `/v1/conversations/{conversation_id}/messages` | Paginated history / synchronous send or reply |
| PUT | `/v1/conversations/{conversation_id}/read-state` | Update own monotonic read state |
| GET / PATCH / DELETE | `/v1/messages/{message_id}` | Read / edit / tombstone globally identified message |

Registration remains a registered route when closed and returns
`403 registration.closed`. It reuses `auth.registration`, not an HTTP-specific
toggle. All discovery/conversation/message routes require bearer authentication.
Dependent collections are nested; globally identifiable message items have
top-level routes. There is no unrestricted user listing, session listing or health
endpoint. Axum also supplies the normal implicit HEAD handling for GET.

## Client messaging journey

1. Authenticate with `POST /v1/sessions` and retain the returned access/refresh
   credentials securely. Use bearer access authentication for subsequent steps.
2. Discover a peer with `GET /v1/users?username=bob`. Exact canonical username
   matching returns at most one `{id, username}` item; missing/disabled users both
   return an empty page. Invalid usernames return `422 user.invalid_username`.
3. Open a Direct using `POST /v1/conversations` with
   `{"kind":"direct","participant_id":"<user-id>"}`. A fresh pair returns 201;
   an existing unordered pair returns 200. Self Directs are supported.
   A Group uses `{"kind":"group","title":"Engineering","participant_ids":[]}`
   and returns 201. The variants reject unknown/mixed fields.
4. Fetch `GET /v1/conversations?limit=50` to render an inbox in one request.
   Every item includes title, own role/read marker, unread count and the last
   enriched message. Group titles are stored; Direct titles project the current
   peer username (own username for self Directs), not persisted title metadata.
5. Fetch `GET /v1/conversations/<id>/messages?limit=50` for newest-first history.
   Each message includes sender ID/username and an optional shallow reply preview;
   clients do not need per-message sender/reply lookups. Disabled historical
   identities remain attributable. Tombstones contain null content and deleted_at.
6. Send synchronously with `POST /v1/conversations/<id>/messages` and
   `{"content":"hello"}`. To reply, add `"reply_to_message_id":"<message-id>"`.
   A fresh send returns 201, not 202. Reply targets must be active and in the same
   conversation; unavailable targets return `422 message.reply_target_unavailable`.
7. Update `PUT /v1/conversations/<id>/read-state` with
   `{"last_read_message_id":"<message-id>"}`. The 200 response contains the
   authoritative marker and unread_count from the same transaction. Older/same
   targets succeed without regression, including when another device advanced it.

Message item reads/edits/deletes need only their global MessageId. Exact-content
edits are successful no-ops. Deletion returns 200 with the structural tombstone;
authorized repetition preserves deleted_at. Reply previews reflect current target
state and never nest another reply. Only authors edit; authors and Group
Owners/Admins may delete. Normal message deletion is not physical hard deletion.

Group management uses bodyless PUT member (201 added, 200 already present),
PATCH member `{"role":"admin"}` or `{"role":"member"}` (200), DELETE member
(204), PATCH conversation `{"title":"New title"}` (200), and PUT owner
`{"user_id":"<existing-member-id>"}` (200 ConversationResponse). Ownership
transfer demotes the old Owner to Admin; self-transfer is a successful no-op.
Only the Owner transfers ownership. Owners/Admins manage membership and title.
An Owner cannot leave with remaining members until ownership is transferred;
an owner-only leave hard-deletes the Group and its messages. Direct lifecycle
mutations return `409 conversation.operation_not_supported`.

Missing/inaccessible conversations collapse to `404 conversation.not_found`;
missing/inaccessible global messages collapse to `404 message.not_found`.
Known members lacking role permissions receive
`403 conversation.insufficient_permissions`, `403 message.not_author` or
`403 message.insufficient_permissions`. Missing existing-member targets return
`404 conversation.member_not_found`; Owner protection and required ownership
transfer return 409 with `conversation.owner_protected` and
`conversation.ownership_transfer_required`. Editing tombstones returns
`409 message.deleted`. Invalid titles/content and over-policy content return 422
with `conversation.invalid_title`, `message.invalid_content` and
`message.content_too_large` respectively.

New memberships atomically require existing enabled users, including Group
creators; unavailable participants return 422 without distinguishing missing from
disabled. Existing disabled memberships/Directs remain valid and idempotent adds
do not reset read state. Other members' read markers are never exposed. Own
watermarks are unread-calculation boundaries, **not read receipts**.

## Collection pagination and activity

All collection responses are `{ "items": [], "next_cursor": null, "has_more": false }`.
`has_more` equals cursor presence. Query parameters are `limit` (default 50,
inclusive range 1..100) and opaque next-only `cursor`; no offset, page number or
client-controlled sorting is supported. Exact username lookup never issues a
cursor; supplied discovery cursors are rejected.

- Inbox: effective activity DESC, ConversationId DESC; cursor bound to actor.
- Members: joined_at ASC, UserId ASC; cursor bound to conversation.
- Messages: MessageId DESC; existing message v1 cursors remain compatible.

Inbox activity derives from `max(conversation.updated_at, created_at of the
highest-MessageId message)`, falling back to updated_at when empty. Real Group metadata/membership
changes and sends may raise a conversation. Edits, tombstoning and reading do not
manufacture new activity. The last message can be a tombstone. No activity column
is promised. Continuation uses normal keyset semantics, not frozen snapshots:
activity raised above an old boundary is obtained by refreshing page one.

Malformed query structures return `400 request.invalid_query`; out-of-range
limits return `422 pagination.invalid_limit`; malformed/wrong-scope cursors return
`400 pagination.invalid_cursor`. Malformed UUIDs in path parameters return
`400 request.invalid_path`; malformed UUIDs in JSON identifier fields return
`400 request.invalid_identifier`. Each page revalidates current membership.

## Optional idempotent creation

`Idempotency-Key` is supported only on conversation creation and message sends.
It is optional, must occur once, and contains 1..128 visible ASCII bytes, preserved
exactly without trimming. Invalid/duplicate values return
`400 idempotency.invalid_key`; raw keys are neither persisted nor logged.

The 24-hour guarantee scopes keys to actor + operation + resource (actor for
conversation creation; conversation for sends). Fingerprints use semantic input,
not JSON formatting/order. Group participants exclude the creator and form a
sorted distinct set; title and message content retain their exact accepted bytes.

An active matching key returns the original resource's current authorized
representation with `200` and `Idempotency-Replayed: true`, not a historical byte
replay. This header is absent on ordinary responses. First keyed access to an
already-existing Direct returns 200 without the replay header, but establishes
the mapping. Changed input returns `409 idempotency.key_reused` without effects.

Only successful creates reserve keys. Resource and record commit together and
survive restarts. A physically removed original yields
`409 idempotency.result_unavailable`, never a replacement before expiry. Records
are active while expires_at is greater than now; at/after expiry keys may be
reused. Tombstoned messages still exist and may be returned by replay. Retention
cleanup is opportunistic SQLite housekeeping, without workers or a Redis adapter.

## Authentication

Tokens are JSON transport data, not cookies. Access tokens are submitted only as
`Authorization: Bearer <access-token>`. Query parameters, cookies, request bodies
and alternative headers are not access-token sources. Multiple Authorization
values are rejected. Bearer scheme matching is case-insensitive.

Missing, invalid, expired and otherwise rejected access credentials produce
`401 auth.invalid_access_token`. Operational verification failures produce a
generic 500. The extractor verifies the token's signature and claims without
consulting identity or session storage. All authentication 401 responses include
`WWW-Authenticate: Bearer` for access-token authentication failures, without credential/error internals.

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
URIs. Authorization/Cookie/Set-Cookie/Idempotency-Key are marked sensitive. HTTP tracing never
logs request/response bodies or bearer values. Successful lifecycle events use
user/session IDs inside the correlated request span.

## OpenAPI and Swagger

OpenAPI 3.1 is generated from the same Utoipa-aware composition that registers
runtime routes. Operation IDs and schema names are intentional public contracts.
`Users`, `Sessions`, `Conversations` and `Messages` tags organize the operations.
`bearerAuth` protects discovery, messaging, current-user retrieval and logout;
registration/login/refresh remain public credential operations.

Response headers are part of the generated contract: every declared response
includes `X-Request-Id` (UUID), access-token authentication failures include `WWW-Authenticate: Bearer`,
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
and Idempotency-Key request headers and GET/POST/PUT/PATCH/DELETE methods are permitted. Valid preflights are
handled before authentication and quotas. Allowed-origin API errors retain CORS
headers, and browsers can read `X-Request-Id`, `Retry-After` and `Idempotency-Replayed`.

## Rate limiting

Governor token buckets are in-memory, per process and per immediate peer IP.
The general quota covers API requests, with an additional shared stricter quota
for POST registration/login/refresh. Current-user and logout use only the general
quota, as do discovery and messaging. Documentation assets spend neither quota. Inactive keys are cleaned
opportunistically every 1024 API quota checks; no background worker is started.

Forwarded, X-Forwarded-For, X-Real-IP and vendor IP headers are ignored. Behind
a reverse proxy, the limiter sees the proxy as peer: adjust/disable quotas until
explicit trusted-proxy support is delivered. This is not a distributed limiter.

The future serving runtime must use Axum's connect-info make-service, supplying
`ConnectInfo<SocketAddr>`. A missing peer with limiting enabled is an internal
configuration error (500), never an unlimited request or a forwarded-IP fallback.

See [server configuration](server-configuration.md) for defaults and overrides.

## Current limitations

No listener, health/readiness, WebSocket, events/workers, read receipts, display
names, attachments, reactions, threads, search, cookies, CSRF mode, OAuth/OIDC,
trusted proxies, distributed quotas or instant JWT revocation is implemented.
