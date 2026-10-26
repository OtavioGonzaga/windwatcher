# Conversations

## Participant eligibility

Creating a new membership requires an existing enabled user, including the Group
creator and initial participants. Persistence checks eligibility atomically with
insertion; the create/add use cases depend only on `ConversationStore`, not on
an identity preflight. Missing and disabled participants are publicly represented
as `ParticipantUnavailable`.

Disabling a user does not erase existing membership or identity attribution.
Opening an existing Direct or adding an existing Group member still succeeds,
even when that peer/member is disabled, without resetting read state.

## Consumer queries

`ListConversations` and `GetConversationSummary` return `ConversationSummary`:
metadata, actor-relative title, own membership/read marker, unread count and the
highest-ID `MessageView` (including tombstones). A Group title is persisted;
a Direct title is the peer username, or the actor's username for self Directs.
Disabled existing peers retain attribution. Direct titles are not entity metadata.

The inbox sorts by effective activity DESC, ConversationId DESC. Effective
activity is `max(conversation.updated_at, MAX(message.created_at))`, falling back
to `updated_at` for empty history. Sends and real Group metadata/membership
mutations can raise activity. Message edits/deletes and read updates do not.
Activity is derived from primary state, not a persisted `last_activity_at` column.

`ListConversationMembers` supports both kinds, sorting by joined_at ASC, UserId
ASC. It includes disabled existing users but never exposes other members' read
markers. Every page rechecks current actor membership in the read snapshot.

## Conversation kinds

Windwatcher currently supports only `Direct` and `Group` conversations.

## Direct conversations

A Direct is identified by its unordered semantic participant pair. Starting A/B
and B/A returns the same Direct. A Direct may represent one user talking to
themselves: `Direct(A, A)` is valid and has one membership row. Directs have no
title, their participants are immutable, and they cannot be renamed or left.

## Group conversations and memberships

A Group has a required nonblank title and exactly one Owner. It may contain only
its Owner, both when created and after all other members leave. Additional
members initially have the Member role. Roles are Owner, Admin, and Member.

Owners can manage all non-Owner members and transfer ownership. Admins can
rename, add members, manage Members and Admins, but cannot act against the
Owner. Members may leave. An Owner with other members must transfer ownership
before leaving; a sole Owner leaving hard-deletes the Group, its memberships and
all messages, including messages connected by replies.

Each membership exposes `last_read_message_id`, an optional monotonic UUIDv7
read boundary. Initial memberships have no marker. Adding a new Group member
atomically initializes it to the maximum persisted message ID (including
tombstones), or None for empty history. Adding an existing member does not reset
it. Leaving removes read state; rejoining takes a fresh history boundary.
See [messages](messages.md) for explicit marking and unread semantics.

## Persistence invariants

Conversation storage enforces one Direct per canonical pair, one membership per
user/conversation, and at most one Group Owner. Creation and ownership lifecycle
changes are transactional. Access to an individual conversation requires current
membership.

## Current limitations

There is no conversation listing, invitations, public groups, or network
endpoints yet. Message history uses [opaque cursor pagination](pagination.md).
