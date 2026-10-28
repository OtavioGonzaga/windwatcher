# Domain events

Successful mutating application operations return `Outcome<T>`. Its value is the
operation-specific result and its `DomainEvent` list records meaningful facts
committed by that operation. The wrapper has private fields and exposes borrowed
value/event access plus consuming value/parts access.

`Outcome<T>` deliberately does not behave transparently as `T`; callers must
explicitly decide how to consume the committed value and emitted facts.

Events are constructed only after primary persistence returns a successful
committed outcome. Failures return `Err`, while no-op mutations, semantic Direct
reuse and idempotency replays return no events. The vector order is the operation's
emission order only; it is not a cross-request or durable ordering guarantee.

Events are typed, in-process Rust values owned by the `windwatcher` library.
They are produced after primary persistence succeeds. They are not serialized,
persisted, replayable, audit records, realtime payloads or queue jobs.

## Event catalog

- `ConversationCreated`: a Direct or Group was newly created.
- `GroupRenamed`, `GroupDeleted`: a Group title changed or its sole owner deleted it.
- `GroupMemberAdded`, `GroupMemberRoleChanged`, `GroupMemberRemoved`, and
  `GroupOwnershipTransferred`: committed membership lifecycle facts.
- `MessageSent`, `MessageEdited`, and `MessageDeleted`: carry the committed
  transport-independent `MessageView` projection.
- `ConversationReadStateAdvanced`: carries the affected user, conversation and
  authoritative read state; it is not a read receipt.

Conversation creation, membership and Group lifecycle operations emit the matching
facts only when their semantic storage outcome changed. Send/edit/delete/read-state
operations follow the same rule. `CreateConversation` and `SendMessageCommand`
emit only for `CreateDisposition::Created` and suppress events for replays.

The standalone server reacts to these events after the operation completes and
maps them to its unchanged WebSocket contract. Reactions are best effort; a
reaction failure does not undo a committed operation. A future durable outbox or
job representation would be separate from this model. There is no event store,
event bus, replay API, integration-event representation, queue or worker here.
