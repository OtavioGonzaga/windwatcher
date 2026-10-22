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

## Current limitations

This domain slice does not yet implement credentials, registration, sessions,
authentication, authorization, persistence, or HTTP contracts.
