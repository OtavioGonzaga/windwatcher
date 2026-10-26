# Messages

## Message model

`Message` is an active text message or a structural deletion tombstone. It
preserves a UUIDv7 `MessageId`, conversation, historical sender, optional reply
target, creation time, optional last edit time, and optional deletion time.
Active messages have content and no deletion time; tombstones have a deletion
time and no content. `Message::restore` validates these combinations and rejects
self replies. There are no arbitrary public setters.

## Message content

`MessageContent` represents intrinsically portable text. It rejects only embedded
NUL (`U+0000`), because normal text storage is not equally capable of representing
it across backends. Empty text, whitespace-only text, newlines, and Unicode are
valid. Accepted text is preserved exactly: no trimming or normalization.

## Message policy

`MessagePolicy` is an operator/embedder acceptance policy for **new sends and
edits**, separate from portable representation. The default inclusive maximum is
131072 UTF-8 bytes (128 KiB). Byte length, not Unicode scalar or grapheme count,
is measured. `MessagePolicy::new(0)` accepts only empty text. The application
validates content before calling storage; adapters do not apply size policy.

Reducing the policy does not invalidate historical messages: a 100 KiB message
can still be loaded after switching to a 64 KiB limit. A future edit must satisfy
the policy active at that time.

## Sending

`SendMessage::new(&storage, &policy).execute(actor, conversation_id, text,
reply_target, now)` generates an ID and returns the persisted message only after
primary persistence succeeds. The actor is always the sender. Current membership
is required for Direct, self Direct, and Group conversations. Sending never
advances the sender's read marker and never changes conversation metadata time.

## Replies

Send/get/edit/delete and paginated history return `MessageView`: the domain
message, sender username and an optional shallow `ReplyPreview`. These are
projections loaded together, without per-message identity lookups. Disabled
senders retain their username. Reply previews show the target's current text or
content-free tombstone, and never contain another nested reply.

A new reply must target an existing active message in the same conversation.
Missing or cross-conversation targets are unavailable; a tombstone cannot receive
new replies. Storage enforces this atomically with insertion, including against
concurrent deletion. If the reply commits first, it survives later target
deletion and continues referencing that tombstone. This lightweight reference is
not thread support.

## Editing

`GetMessage`, `EditMessage` and `DeleteMessage` address items by globally unique
`MessageId` alone: `execute(actor, message_id, ...)`. The store resolves the
conversation and checks current membership in the same logical snapshot or write
transaction. Missing items return `MessageNotFound`; inaccessible existing items
remain a distinct internal `NotMember` outcome for transport-level privacy mapping.

`EditMessage` requires current membership and authorship, even for a Group Owner
or Admin. An exact content match returns `EditMessageOutcome::Unchanged` without
changing `edited_at`. A meaningful edit replaces content and sets `edited_at` to
the supplied time; identity, sender, conversation, reply and creation time remain
unchanged. Deleted messages cannot be edited. No edit history is retained.

## Deletion and tombstones

`DeleteMessage` does not remove message identity. It destroys content in primary
state and returns a tombstone, preserving all structural metadata and any prior
edit time. Existing replies remain valid. Repeated authorized deletion returns
`AlreadyDeleted` without rewriting the first deletion time. Windwatcher has no
deleted-content retention, restoration or undelete API.

This is logical content removal, not a forensic erasure guarantee: SQLite pages,
WAL files and operator-managed backups can retain old bytes. Backup/media
retention is outside this message lifecycle contract.

## Authorization

Every operation requires current membership. Only authors may edit. Authors may
delete their own messages. Group Owners/Admins may additionally delete other
users' messages, including historical messages whose sender has left. A Direct
peer or regular Group Member cannot moderate another author's content. Permission
is checked before returning `AlreadyDeleted`; tombstones do not bypass it.
Mutating store operations enforce membership and role at persistence time.

## Read state

`ConversationMember::last_read_message_id` is an explicit monotonic UUIDv7
watermark. `MarkConversationReadThrough` accepts an existing same-conversation
message, including a tombstone. A greater target advances it; equal/older targets
return `Unchanged`. Marking a missing or cross-conversation target fails.

Both changed and unchanged outcomes carry `ConversationReadState`; `.state()`
returns the authoritative effective marker and unread count from the same write
transaction. An older target from another device never regresses the marker.
These watermarks are not receipts: Group join initializes historical read state
without the user explicitly reading that history.

Sending does not mark intervening messages as read. Read markers advance only
through explicit mark-read after membership initialization. Direct and initial
Group memberships start with no marker. Adding a new Group member atomically
initializes their marker to the current maximum persisted message ID, including
tombstones, or None for empty history. Already-member adds preserve the marker.
Leaving removes membership/read state; rejoining takes a new history boundary.

## Unread counts

`GetUnreadCount` returns a checked `u64` derived from primary messages and
membership. Count active messages from other senders whose IDs exceed the read
marker, or all active non-own messages if the marker is absent. Own messages and
tombstones never count. Deleting an unread message reduces the count without
changing the marker; deleting the marker's target does not invalidate it.
There is no persisted unread counter.

## Ordering

`MessageId::Ord` is the canonical v1 ordering key. IDs generated by the process
through `MessageId::new()` are monotonic in generation order. Persistence must
preserve equivalent UUID byte/value ordering. There is no separately coordinated
per-conversation sequence or globally serialized commit order across independent
producers. Even within a process, concurrent persistence may commit in a different
order from ID generation. Read boundaries are ID boundaries, not commit offsets;
a later insertion with an ID below a watermark does not become unread.

## Persistence behavior

The executor-independent `MessageStore` port owns semantic atomic send, get, list,
edit, delete, mark-read and count operations. Use cases expose their own focused
errors/outcomes rather than store outcomes. SQLite implements this port with
same-conversation reply integrity, tombstone checks and write transactions that
acquire the writer lock before checking authorization. No message/read operation
updates `Conversation.updated_at`. Deleting an Owner-only Group cascades all
messages, including reply relationships. See [persistence](persistence.md) and
[ADR 0008](adr/0008-message-lifecycle-and-read-state.md).

## Current limitations

This delivery has no HTTP, realtime, events, queues, attachments, reactions, threads,
search, edit history, undelete or message purge.

## Paginated history

`ListMessages::new(&storage).execute(actor, conversation_id, PageRequest::default())`
returns `Page<MessageView>` newest-first by UUIDv7 `MessageId::Ord`. Active messages
and tombstones both appear. `next_cursor()` continues toward older history using
`PageRequest::new(limit).with_cursor(cursor)`; the last page has no cursor even
when exactly full. Listing does not mark read or apply the current write policy.

Every page requires current membership, including Direct and self Direct.
Leaving a Group invalidates access with an earlier cursor; deleting the
conversation yields not-found. Cursor scope validation does not replace
authorization. See [pagination](pagination.md) for limits, cursor opacity,
versioning and consistency between requests.
