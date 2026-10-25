# Conversations

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
before leaving; a sole Owner leaving hard-deletes the Group and its memberships.

## Persistence invariants

Conversation storage enforces one Direct per canonical pair, one membership per
user/conversation, and at most one Group Owner. Creation and ownership lifecycle
changes are transactional. Access to an individual conversation requires current
membership.

## Current limitations

There are no messages, read state, pagination, invitations, public groups, or
network endpoints yet.
