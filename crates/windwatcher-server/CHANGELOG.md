# Changelog

All notable user-facing changes to this package are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- HTTP v1 router with public registration, local login, refresh rotation,
  idempotent current-session logout and current-user retrieval; no listener yet.
- Server-owned transport DTOs, RFC 9457 problems with stable codes, UUIDv7 request
  correlation, bearer extraction, safe structured tracing and no-store grants.
- Code-first OpenAPI 3.1 at `/openapi/v1.json` and vendored Swagger UI at `/docs`.
- OpenAPI response-header contracts for correlation IDs, no-store token grants,
  bearer challenges and retry delays, plus UUID formats for transport IDs.
- Validated configurable deny-by-default CORS and general/authentication peer-IP
  governor quotas, including `config check` and environment overrides.
- Bounded Tokio blocking adapter preserving the engine's Argon2 semantics and
  zeroization, including cancellation-safe concurrency permits.

- `[messages].max_content_bytes` configuration resolving to the library's
  new-write policy (131072 UTF-8 bytes by default), including environment
  overrides and full config validation.
- Message storage delegation and explicit migration support for message/read state.

- Server CLI foundation with layered TOML/environment configuration, user/system
  runtime paths, stderr tracing, explicit SQLite migration commands, configuration
  validation, and trusted local-user provisioning.

- Enable the library's optional HS256 JWT access-token adapter for future server wiring.
