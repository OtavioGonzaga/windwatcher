# WebSocket realtime transport

The database and HTTP API are authoritative. WebSocket delivers **best-effort
state-change notifications**, not an ordered event log, queue, snapshot, or
replacement for HTTP synchronization. A disconnected client can miss events.

## Connect and authenticate

1. Authenticate normally and obtain an access token.
2. `POST /v1/realtime/tickets` with `Authorization: Bearer <access-token>`.
3. Connect to `wss://<host>/v1/realtime?ticket=<ticket>`.
4. Reconcile inbox/current state through HTTP, then continue processing frames.

Never put the access JWT in a WebSocket URL. Browsers cannot set an ordinary
Authorization header on their WebSocket connection; the query ticket is an
explicit credential, not ambient cookie authentication. CORS is not a WebSocket
handshake authorization policy. Complex Origin/proxy policy is not implemented.

Ticket creation returns `201 Created` with `Cache-Control: no-store`:

```json
{
  "ticket": "<43-character-base64url-secret>",
  "expires_at": "2026-10-27T16:30:30Z"
}
```

Tickets contain 32 cryptographically random bytes, are base64url without padding,
expire after 30 seconds by default (capped by session expiry), and are one-use,
user-bound, session-bound and process-local. Only SHA-256 hashes are retained in
memory. Expired entries are cleaned opportunistically. A normal GET without valid
upgrade headers does not consume a ticket; concurrent upgrades have one winner.
Tickets and access tokens must never appear in logs, including proxy/access logs.
The application traces matched routes rather than URI query strings.

Both endpoints use the general `/v1` rate limiter, not the stricter login quota.
Ticket issuance requires valid Bearer access plus an active persisted session.
Preparation revalidates the session and registers the connection before `101`.
No `connection.ready` application event is required.

| Upgrade result | HTTP status / Problem Details code |
| --- | --- |
| Connected | `101 Switching Protocols`, no JSON response body |
| Missing, duplicate or malformed query / unsupported query parameter | `400 request.invalid_query` |
| Unknown, expired or consumed ticket | `401 realtime.invalid_ticket` |
| Missing, revoked, expired or wrong-owner session | `401 realtime.session_inactive` |
| Not a valid WebSocket upgrade | `426 realtime.upgrade_required` |
| Peer quota exceeded | `429 rate_limit.exceeded` |
| Internal failure | `500 internal.error` |

Ticket-authenticated upgrade failures do not include a Bearer challenge. On ticket
creation, only access-token authentication failures carry `WWW-Authenticate:
Bearer`; inactive sessions do not. Invalid tickets never disclose prior existence.

## Envelope and compatibility

Every application frame is UTF-8 JSON Text:

```json
{
  "event_id": "019a2890-0000-7000-8000-000000000001",
  "type": "message.created",
  "schema_version": 1,
  "occurred_at": "2026-10-27T16:30:00Z",
  "payload": {}
}
```

`event_id` is a server UUIDv7 identifying an emission for correlation. It is **not**
a sequence, cursor, checkpoint, ordering key, database commit position or replay
offset. `occurred_at` is informational RFC 3339 UTC, normally the mutation time;
it is not an ordering guarantee. All initial event schemas have version 1.

Adding optional fields or event types is compatible. A breaking payload change or
reinterpretation increments that event type's schema version. Removing or renaming
an existing event type is an external API break. Clients should ignore unknown
types, versions and fields, optionally reconciling through HTTP; unknown data does
not require closing the socket. Event schemas are also registered in OpenAPI, but
this document is the authoritative frame contract; no AsyncAPI toolchain is used.

## Event catalog

### Messages

`message.created`, `message.updated`, and `message.deleted` contain a complete
current `message`, matching the HTTP `MessageResponse` consumer shape:

```json
{
  "message": {
    "id": "019a2890-0000-7000-8000-000000000002",
    "conversation_id": "019a2890-0000-7000-8000-000000000003",
    "sender_id": "019a2890-0000-7000-8000-000000000004",
    "sender_username": "alice",
    "content": "hello",
    "reply_to": null,
    "created_at": "2026-10-27T16:30:00Z",
    "edited_at": null,
    "deleted_at": null
  }
}
```

Updates carry full projections, not patches. Deletion carries the full tombstone:
`content: null` and the original deletion timestamp in `deleted_at`. Reply previews
remain shallow current HTTP previews (`id`, `sender_id`, `sender_username`,
`content`, `deleted_at`), including tombstones. Message parity is contract-tested.

### Conversations

| Type | Payload |
| --- | --- |
| `conversation.created` | `{"conversation_id":"…","kind":"direct"}` or `"kind":"group"` |
| `conversation.updated` | `{"conversation_id":"…","title":"Engineering","updated_at":"2026-10-27T16:30:00Z"}` |
| `conversation.deleted` | `{"conversation_id":"…"}` |

Creation deliberately does not embed an actor-relative `ConversationResponse`.
Fetch `GET /v1/conversations/{id}` for the recipient's title, own membership, own
watermark, unread count and latest message. Initial participants receive only
creation, not additional member-added events. Updates describe meaningful Group
metadata changes; Directs cannot be renamed. Deletion currently means the sole
Group Owner left and the Group was hard-deleted.

### Members and ownership

`conversation.member.added` and `conversation.member.updated` use:

```json
{
  "conversation_id": "019a2890-0000-7000-8000-000000000003",
  "member": {
    "user": {"id": "019a2890-0000-7000-8000-000000000004", "username": "bob"},
    "role": "member",
    "joined_at": "2026-10-27T16:30:00Z"
  },
  "conversation_updated_at": "2026-10-27T16:30:00Z"
}
```

Role changes include the final role. Removal (including a Member/Admin leaving)
does not require the removed membership to remain present:

```json
{
  "conversation_id": "019a2890-0000-7000-8000-000000000003",
  "user_id": "019a2890-0000-7000-8000-000000000004",
  "conversation_updated_at": "2026-10-27T16:30:00Z"
}
```

Atomic ownership transfer emits one `conversation.owner.transferred`, not two
independent member updates:

```json
{
  "conversation_id": "019a2890-0000-7000-8000-000000000003",
  "previous_owner": {
    "user": {"id": "019a2890-0000-7000-8000-000000000004", "username": "alice"},
    "role": "admin",
    "joined_at": "2026-10-27T16:00:00Z"
  },
  "new_owner": {
    "user": {"id": "019a2890-0000-7000-8000-000000000005", "username": "bob"},
    "role": "owner",
    "joined_at": "2026-10-27T16:00:00Z"
  },
  "conversation_updated_at": "2026-10-27T16:30:00Z"
}
```

The roles represent the successful atomic mutation, not two subsequent role
lookups. Attribution is hydrated after commit; an unavailable/replaced membership
can make publication fail best effort rather than produce a partial event.
`conversation_updated_at` can help avoid obvious stale UI updates but is not a
global version number or snapshot boundary.

### Own read state

`conversation.read_state.updated` carries the authoritative state returned by the
mark-read operation:

```json
{
  "conversation_id": "019a2890-0000-7000-8000-000000000003",
  "last_read_message_id": "019a2890-0000-7000-8000-000000000002",
  "unread_count": 0
}
```

The watermark may be null in the schema. This is unread-state synchronization
across the user's devices, **not a read receipt**. Peers never receive this event
or another member's read watermark.

## Audience and publication

- Normal conversation/message/member/ownership notifications resolve all current
  member IDs at dispatch-time lookup, without public pagination or per-user queries.
- Removal reaches remaining members **plus the removed/leaving user**, who can
  remove the conversation locally; subsequent HTTP access is naturally unavailable.
- Sole-Owner deletion reaches that former Owner.
- Read-state notifications reach only the actor's connections.
- Every connected socket of a recipient receives the event, including the device
  that initiated the HTTP mutation. Unconnected users do not create delivery work.

Current membership takes precedence over reconstructing a historical audience. A
member removed between commit and recipient lookup may miss a normal event; a
member added during that interval may receive a notification for readable state.
Existing disabled members are included in the storage lookup; session revocation
independently controls connection eligibility.

Handlers submit notifications only after successful primary commit and only for real changes.
Existing Directs, idempotency replays, same-title/role/content updates, existing
member additions, self ownership transfer, repeated deletion, and older/equal read
watermarks emit nothing. Failures emit nothing. Notifier failures are logged and
swallowed: successful HTTP operations never become fake rollbacks. There is no
atomic transaction spanning database effects and fan-out. Process exit or request
cancellation after commit may lose a notification.

Submission reserves task capacity without waiting, before spawning. Projection
hydration, audience lookup, serialization and publication run outside the HTTP
response path. At most `dispatch_concurrency` jobs run (default 32); excess
notifications are dropped and logged, not queued behind waiting tasks. A separate
`dispatch_timeout` (default 5 seconds) bounds the whole job, including hydration.
It does not reuse `heartbeat_timeout`. These limits apply to notification work,
not connection count, and introduce no durable job/retry mechanism.

## Connections and control frames

Multiple connections and multiple sessions per user are supported. Removing one
connection never unregisters another. There are no client application commands:
send mutations through HTTP. Native Ping, Pong and Close are supported; client
Text/Binary application frames close the connection with unsupported-data semantics.

Defaults are Ping every 30 seconds and a 10-second matching-Pong timeout. Each
connection has a bounded 32-event outgoing queue. A full queue stops further
delivery to that socket and closes it, without blocking HTTP or healthy sockets.
An event is serialized once and shared for fan-out. Control/close signals are not
queued behind application data. Registration is guarded and cleaned on cancellation,
failed upgrade, normal exit or panic.

| Close code | Meaning |
| --- | --- |
| `1000` | Normal close |
| `1001` | Heartbeat timeout |
| `1003` | Unsupported client application data |
| `1008` | Session expired, revoked or otherwise inactive |
| `1011` | Internal runtime/storage failure |
| `1013` | Slow consumer / temporary overload |

No JSON event precedes an authorization close. Access JWT expiration alone does
not end an established connection: the underlying session is its authority.
Session expiry has a connection deadline. Local logout invalidates outstanding
tickets and closes all pending/active sockets of that session, not other sessions.
Handshake activation is coordinated with local revocation. Persistence rechecks
every 30 seconds by default detect external/CLI revocation; inability to consult
storage fails closed. There is no instantaneous cross-process revocation promise.

## Reconnect, ordering and client merge

**The realtime connection has no durable replay boundary. Reconnection must be
followed by HTTP reconciliation.** Obtain a fresh ticket, open the socket, then
resynchronize inbox/history/current state through HTTP. Initial connection follows
the same sequence. Neither establishes a durable snapshot boundary.

There are no ACKs, `last_event_id`, `resume_from`, `after_event`, persistent event
records or replay. Unknown query parameters are rejected. There is no exactly-once
promise: tolerate duplicates and merge by resource identity. A socket preserves
frame write order, not global, per-user or per-conversation commit order. Neither
UUIDv7 nor timestamps establish such an order.

Suggested message handling:

- `message.created`: insert/upsert by MessageId.
- `message.updated`: update an existing active message.
- `message.deleted`: retain the tombstone as terminal current state.

Do not let an ambiguously ordered creation/update resurrect a known tombstone.
Refetch message/history or conversation state when ordering is uncertain. HTTP
state wins. These recommendations do not introduce stronger delivery guarantees.

## Deployment boundary

Tickets, hub and fan-out are **in-memory and process-local**. Ticket issuance and
upgrade must reach the same process, and only locally connected recipients see
local notifications. Multiple processes require a future shared signaling design;
there is no Redis, queue reuse, distributed quota or speculative fan-out adapter.
This PR does not provide `serve`, production listener lifecycle, health/readiness
or graceful process shutdown. Test listeners bind only temporary loopback ports.

`RealtimeEvent` lives entirely in `windwatcher-server` and is not `DomainEvent`.
Future domain-event reactions can replace explicit post-commit handler hooks via a
server-side mapper without equating their names/payloads or changing this external
protocol. See [ADR 0012](adr/0012-websocket-realtime-transport.md).
