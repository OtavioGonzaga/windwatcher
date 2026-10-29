# Pagination

Encoded continuation cursors have an inclusive maximum of 1024 bytes
(`MAX_CURSOR_BYTES`). All three v1 cursor kinds (conversations, members and messages)
are checked before Base64 allocation or JSON decoding in the library. Oversized
tokens produce `CursorError::Malformed`; HTTP preflights the same bound and returns
`400 pagination.invalid_cursor`. The limit does not change issued v1 cursor formats.

## Inbox and member continuations

Conversation cursors bind the actor and exclusive descending
`(effective activity, ConversationId)` boundary. Member cursors bind the
conversation and exclusive ascending `(joined_at, UserId)` boundary. Full
timestamp precision is preserved. Both use strict versioned base64url payloads
without padding; possession never replaces membership authorization.

There is no shared snapshot between pages. Newly raised activity above an old
inbox boundary is obtained by refreshing page one; continuation traverses older
keys rather than restarting. Mutable sort keys do not provide frozen-inbox
guarantees. Future result-changing filters must also bind their cursors.

Exact enabled-username discovery returns at most one item in the same `Page`
shape and never issues a continuation. No all-user listing is introduced.

## Strategy

Windwatcher uses opaque cursors with keyset pagination and next-only navigation.
`ListMessages` is the first consumer. Pagination is a library contract, independent
of HTTP, query parsing, serialization DTOs, and any database driver.

## Page requests

`PageRequest::default()` requests the first page with 50 items.
`PageRequest::new(limit)` supplies a validated `PageLimit`;
`.with_cursor(token)` continues an earlier response. A present but empty token is
invalid, not a request for the first page. Page size may change between requests:
the cursor identifies a boundary, not the previous size.

## Page responses

`Page<T>` exposes `items()`, `into_items()`, `next_cursor()` and `has_more()`.
`has_more()` is derived from cursor presence. The final page has no cursor, even
when its length exactly equals the requested limit. Empty history is successful.
There are no totals, page numbers, arbitrary page jumps, or previous cursors.

## Limits

`PageLimit` accepts `1..=100`, defaults to 50, and rejects zero or oversized values
with `PageLimitError`. It never silently clamps. Store ports receive the validated
type, not an unchecked integer. These bounds are not server configuration.

## Opaque cursors

A cursor may currently be implemented as base64url-encoded data, but consumers
must not decode, inspect, construct or modify it. Encoding uses the URL-safe
alphabet without padding. The decoded representation is private, not a consumer
contract. Cursors are self-contained: no persistence, registry or cache is needed.
They do not expire and have no revocation mechanism.

V1 is unsigned and unencrypted: opaque does not mean secret or tamper-proof.
Unsigned does not mean trusted. Cursor possession is not authorization; every
operation independently enforces current membership. An authorized caller who
alters a valid boundary can move within their own accessible history, not gain
access to another resource.

## Cursor versioning

The private payload is versioned, initially v1. Unsupported versions return a
typed `CursorError`; malformed encoding, payloads or identifiers fail cleanly
without including the supplied token in errors. Strict payload decoding rejects
unknown fields. Internal compatibility fixtures preserve decoding of issued v1
tokens. Future formats require deliberate version handling, not silent changes.

## Resource binding

Message cursors identify the messages resource and one conversation scope.
Passing a token for another resource or conversation returns `InvalidCursor`
before storage. This binding catches accidental reuse; it does not grant access.

## Keyset semantics

The application decodes a message cursor into an internal `MessageId` boundary.
Storage lists only IDs strictly below that boundary, in descending order. The
boundary need not reference an existing message. Storage returns items and a
lookahead-derived `has_more`; the application encodes the last **returned** ID,
never the lookahead item's ID. Storage never handles external cursor encoding.

SQLite uses the existing `(conversation_id, id)` key and reads `limit + 1` rows.
Conversation existence, current membership, and items share one read transaction,
without an unnecessary writer lock. No pagination migration is required.

## Message ordering

Message pages are newest-first by `MessageId::Ord` (UUIDv7 descending).
`next_cursor` continues toward older history. Tombstones remain in their original
positions. Listing does not mark messages read; use `MarkConversationReadThrough`
explicitly. No timestamp, sequence, rowid or commit-time sort is introduced.

## Concurrent mutations

Pagination is not a multi-request database snapshot. Each page is independently
authorized. Newer IDs inserted above the boundary do not enter older-history
continuations: refresh the first page to see them. UUIDv7 retains the
[ADR 0008](adr/0008-message-lifecycle-and-read-state.md) multi-producer limitation:
a later insertion with a lower ID may appear on a later page.

Messages may become tombstones between requests without changing their position.
A former member receives `NotMember` even with an earlier valid cursor; a deleted
conversation yields `ConversationNotFound`. Rejoining restores normal history
access, subject to current membership. Keysets provide traversal by ordering key,
not snapshot isolation across requests.

## Current limitations

`ListMessages`, `ListConversations`, and `ListConversationMembers` use
`PageRequest`/`Page<T>` with resource-specific ordering and private cursor
semantics. There is no arbitrary sorting, filtering, search, backwards navigation,
total count, signing, encryption, or snapshot cursor.

See [ADR 0009](adr/0009-opaque-keyset-pagination.md).
