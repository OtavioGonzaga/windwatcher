# 0017: Explicit trusted proxies for rate-limit addresses

- Status: Accepted
- Date: 2026-10-29
- Supersedes: only the immediate-peer-only addressing decision in ADR 0010

## Context

Immediate-peer quotas prevent forwarded-header spoofing but make clients behind
a reverse proxy share one bucket. v1 needs an explicit operational opt-in without
adding general proxy identity infrastructure or changing authentication.

## Decision

Own proxy trust entirely in the server's HTTP rate-limit configuration. The
`trusted_proxies` list defaults to empty and accepts explicit IPv4/IPv6 CIDRs.
Only trusted TCP peers may supply a single `X-Forwarded-For` header. Parse at most
4096 bytes and 32 bare IP hops, validate the complete chain, then walk right to
left to the first untrusted IP. Normalize IPv4-mapped IPv6 addresses to IPv4;
operators configure those networks as IPv4 CIDRs. Use the result in both quotas.

Missing, repeated, malformed or excessive headers, and entirely trusted chains,
fall back to the peer bucket. Never use headers in place of missing ConnectInfo.
Ignore `Forwarded`, `X-Real-IP` and vendor headers. No proxy-trust concerns enter
the reusable library, actor authentication, or authorization.

## Consequences

Existing deployments keep peer-based behavior. Operators must trust minimal
networks and configure their proxy to overwrite the header or append the actual
observed peer. Malformed-header fallback can share a bucket but cannot grant an
arbitrary supplied identity. Limits remain per-process, in-memory, and not a
distributed abuse-prevention mechanism. RFC Forwarded parsing is outside scope.
