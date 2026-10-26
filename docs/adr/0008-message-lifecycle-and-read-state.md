# 0008: Message lifecycle and read state

- Status: Accepted
- Date: 2026-10-25

## Context

The conversation foundation needs durable text messages, lightweight replies,
editing, deletion, and per-member read state. The library must define portable
ordering, content acceptance, authorization, referential behavior, and unread
derivation without depending on HTTP, realtime or workers.

## Decision

`MessageId` UUIDv7 is the canonical v1 ordering key; no separate message sequence
is introduced. Adapters preserve `MessageId::Ord`. In-process generation is
monotonic, but neither independent producers nor concurrent insertion are
promised globally coordinated commit order. A read watermark is an ID boundary,
not a commit offset; later inserted lower IDs fall below it.

`MessageContent` preserves exact text, including empty/blank text, rejecting only
NUL for portable text storage. `MessagePolicy` limits UTF-8 byte length for new
sends and edits. Its default inclusive maximum is 131072 bytes (128 KiB); zero
is valid. Changing policy does not invalidate historical reads.

Normal message deletion is structural soft deletion with content destruction:
identity, sender, reply and timestamps survive as a tombstone, without retained
content or undelete. This does not promise forensic erasure from media or
operator-managed backups. Existing replies survive target deletion, while new
replies require an active same-conversation target atomically at persistence time.
SQLite uses a composite reply FK with NO ACTION, allowing conversation-level
cascading deletion of all messages without breaking reply integrity.

Current membership is required for every operation. Only authors edit; authors
and Group Owners/Admins delete. Moderator deletion does not require the original
sender to remain a member. Repeated deletion still requires permission. Exact
edits and repeated deletes preserve timestamps.

Individual get/edit/delete operations use globally unique `MessageId` without a
redundant conversation argument. Persistence resolves the parent and authorizes
within the same snapshot/transaction. Their not-found outcome is message-specific;
conversation cascading deletion also removes the message.

`MessageView` enriches read/mutation results with the current sender username and
shallow reply target state in the same logical persistence operation. Disabled
identities remain attributable and reply tombstones remain visible. Projection
fields are not persisted in the domain entity and replies never recurse.

Read state belongs to membership as `last_read_message_id`. Explicit read markers
are monotonic and may point to tombstones. Sending does not advance read state.
Own messages and tombstones are excluded from unread. Unread count is derived
from active non-own messages after the marker, not persisted. New Group
memberships start read through the current maximum persisted ID, including
tombstones, atomically with insertion. Existing members keep their marker;
rejoining creates a new boundary. Initial memberships start without a marker.

`MessageStore` is an application-owned semantic persistence port. Store outcomes
remain distinct from use-case errors/outcomes. SQLite write operations acquire
the writer lock before reading authorization/targets, preventing stale membership,
reply/delete, edit/delete and read-regression races. Message operations do not
mutate conversation metadata timestamps.

## Consequences

Adapters can share portable lifecycle/read-state contracts without exposing SQL
representations or runtime types. Tombstones preserve structural history and
future paginated lists must include them. UUIDv7 avoids coordinated sequences
with explicitly bounded ordering guarantees. SQLite write contention can still
surface as normalized unavailability; correctness does not depend on workers or
notifications. Pagination, HTTP, events, realtime, retention and attachments
remain separate deliveries.
