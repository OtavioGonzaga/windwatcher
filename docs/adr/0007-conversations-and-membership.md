# 0007: Conversations and membership

- Status: Accepted
- Date: 2026-10-23

## Context

The library needs a portable conversation foundation before messages and network
interfaces. It must support self conversations, mutable Groups, durable access
control, and concurrent creation without exposing database details.

## Decision

Windwatcher has only `Direct` and `Group` conversation kinds. A Direct is an
immutable unordered pair which may contain the same user twice; a self Direct
has one membership. A Group has one Owner and zero or more other members.
Membership roles are `Direct` or `Group(Owner|Admin|Member)`. Initial Group
participants are Members; ownership transfer makes the previous Owner Admin.
A sole Owner leaving hard-deletes the Group, while an Owner with other members
must transfer ownership first.

`ConversationStore` exposes semantic atomic operations rather than generic CRUD
or a Unit of Work. SQLite enforces Direct pair uniqueness and at-most-one Owner;
transactions preserve creation, transfer, and leave lifecycle invariants.

New memberships require existing enabled users atomically at persistence time,
including the Group Owner. Identity preflights are not sufficient and are removed
from create/add use cases. Missing and disabled participants share an unavailable
semantic. Existing Directs and memberships are resolved before eligibility checks;
later disabling does not invalidate existing membership or idempotent adds.

## Consequences

Future adapters share domain and application contracts without inheriting SQL
representations. Direct participants cannot mutate, while Groups can evolve
without allowing a stable zero- or multi-owner state. Messages and read state
remain separate future work.
