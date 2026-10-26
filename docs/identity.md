# Identity

## User

A `User` is a messaging-domain identity. It has a stable UUIDv7 identifier, a
canonical username, creation and update timestamps, and a disabled state. It is
not an instance administrator and does not carry roles or global permissions.

## Username

`Username` is a validated value object. Input ASCII uppercase characters are
canonicalized to lowercase. Canonical usernames are 3 through 32 characters and
contain only lowercase ASCII letters, digits, and underscores (`[a-z0-9_]`).

Whitespace is rejected rather than trimmed. Unicode characters and punctuation
other than underscores are also rejected. The value object validates format; it
does not establish global username uniqueness.

## Disabled state

A new user is enabled. Disabling an enabled user records the supplied timestamp
and updates its modification timestamp. Enabling a disabled user clears that
state and updates the modification timestamp. Repeating either operation when
the user is already in the requested state is a no-op.

## Credentials and persisted state

`User` does not contain authentication material. Local password hashes are
separate values persisted alongside the user through an atomic storage port.
`LocalAuthentication` transfers the user and credential only for authentication;
ordinary user lookups do not expose credentials.

`GetUser` retrieves an identity by ID through `IdentityStore`, distinguishing a
missing identity from an operational storage failure. It returns enabled and
disabled users alike and performs no authorization; callers establish access.

`DisableUser` and `EnableUser` load and persist these domain transitions. Disabling
also revokes every session, including on retries for an already-disabled user.
Re-enabling does not restore revoked sessions. They are trusted operator
capabilities: callers establish trust before invoking them. There are no global
administrator roles. See [local authentication](authentication.md) and
[sessions and tokens](sessions.md) for persistence contracts.
