# v1 security boundaries and hardening review

This is a bounded review of the implemented v1 contracts, not a penetration-test
certification. TLS termination and correct reverse-proxy configuration remain
operator responsibilities. The library owns application authorization; HTTP and
WebSocket transports must not replace it with transport-only checks.

## Controls and regression coverage

| Area | Contract and evidence |
| --- | --- |
| Rate limits | General client-IP token bucket plus shared registration/login/refresh quota; HTTP tests cover auth isolation, Retry-After, preflight bypass and default rejection of spoofed headers. Proxy tests cover explicit CIDR trust, complete bounded chain parsing, mapped addresses and malformed-header fallback. |
| CORS and body size | Deny-by-default, explicit origins or isolated wildcard, no credentials; HTTP tests exercise negative origins and command extraction, including the fixed 2 MiB body limit. |
| Passwords | 8 Unicode scalars minimum for new passwords, 1024 UTF-8 bytes maximum; Argon2id and bounded server blocking work. Password tests cover exact bytes/costs; cancellation tests ensure queued/running work retains its permit. |
| JWT | HS256 only, secret at least 32 bytes, exact issuer and bounded input; explicit expiration tests and HTTP invalid bearer regressions. Access defaults to 15 minutes. |
| Refresh/session lifecycle | Random 256-bit opaque refresh tokens, hash-only persistence, atomic rotation, individual revocation and disabled-user checks. Session and adapter contracts test rotation/revocation; concurrent-refresh regressions require one winner. |
| WebSocket | Canonical 32-byte one-use tickets, hash-only registry, expiry/session checks, bounded socket buffers, heartbeat and revocation. Unit and real-network tests cover concurrent consumption, session isolation, fail-closed storage errors and membership audiences. |
| Group input | Fixed 256-byte new-title and 256-entry initial-participant limits, enforced in both library creation paths; HTTP preflights count before parsing IDs. Tests cover multibyte boundaries, no key reservation and historical-title hydration across adapters. |
| Idempotency | Exact 1..128 visible ASCII key, persisted SHA-256 hash, semantic fingerprint and fixed 24-hour retention. Shared contracts cover scope/conflict/replay, authorization, expiration, deleted results, rejected inputs and atomic effects; adapter tests cover concurrency and physical representations. |
| Redaction | Sensitive request headers, safe route metadata instead of query strings, secret-safe Debug/CLI diagnostics and credential-bearing URL redaction. Sentinel tests cover successful and failed HTTP authentication, cookies, keys, tickets, configuration, network failures and doctor output. Protocol logging cannot be enabled to expose WebSocket secrets. |

### Authorization matrix

HTTP regressions complement shared conversation/message/query/idempotency
contracts and adapter race tests. Authorization must use current membership,
including for a replay or a valid previously issued JWT.

| Operation | Required authority / negative regression |
| --- | --- |
| Conversation detail, members and history | Current member; outsiders and removed members receive unavailable-resource errors. |
| Send, reply, mark read | Current member; removal prevents writes and idempotent replay. |
| Edit | Current member and original author; Owner/Admin cannot edit others' messages. |
| Delete in Direct | Original author and current membership; no moderation role exists. |
| Delete in Group | Author or Owner/Admin; removal/demotion cannot race past committed authorization. |
| Rename, add/remove, assign Admin/Member | Owner/Admin, with Owner protected from removal/demotion. Ordinary Members are denied. |
| Transfer ownership | Owner only, atomically preserving exactly one Owner. |
| Leave | Non-Owner may leave; Owner with peers must transfer first; sole Owner leaving deletes the Group. |
| Realtime audience | Current membership, with explicit terminal removal/leave notifications; session revocation closes only associated sockets. |

Existing memberships survive user disable. Disable prevents new authentication,
refresh and new memberships, and revokes sessions; it does not silently rewrite
the established membership policy. See [sessions](sessions.md) and
[conversations](conversations.md).

## Dependency review — 2026-10-29

Audited the workspace lockfile (405 dependencies) using cargo-audit 0.22.2
and RustSec database commit
`ee1a49a2769a24a596afc1c20cb33748d2769586` (1386 advisories).

The initial scan reported six advisories in the shared TLS dependency graph used
by database/queue clients:

| Package | Lockfile update | Advisories |
| --- | --- | --- |
| rustls | 0.23.45 → 0.23.46 | RUSTSEC-2026-0346, -0347, -0348, -0349 |
| rustls-webpki | 0.103.15 → 0.103.16 | RUSTSEC-2026-0382 |
| webpki-roots | 1.0.9 → 1.0.11 | RUSTSEC-2026-0381 |

These cover TLS secret Debug disclosure, ECH rejection, key-exchange downgrade,
certificate decompression and distrusted intermediates/roots. Some attack paths
require optional TLS features or specific caller behavior; the lockfile uses the
compatible patched releases rather than suppressing findings on that basis.
The repeat scan reported **zero vulnerabilities and zero informational warnings**,
without ignore rules. This result is time-bound; future advisories require a fresh
scan. No CI workflow or dependency-policy framework is introduced here.

Run `cargo audit --json` after updating the advisory database. For focused updates,
use `cargo update -p rustls -p rustls-webpki -p webpki-roots`, then rerun the
repository validation and relevant real adapter contracts. Official network
contracts require disposable services; see [persistence](persistence.md) and
[jobs](jobs.md).

## Deliberate v1 boundaries

- Already-issued access JWTs remain valid until expiration, without a per-request
  session lookup or denylist. Logout/disable prevents refresh and closes realtime.
- Idempotency retention remains fixed at 24 hours.
- Rate limiting is process-local, not persistent or distributed. Trusted proxies
  are opt-in; see [ADR 0017](adr/0017-trusted-proxy-rate-limit-addresses.md).
- There is no WebSocket connection-count policy or shared ticket backend.
- Security CI automation belongs to the subsequent CI delivery.
