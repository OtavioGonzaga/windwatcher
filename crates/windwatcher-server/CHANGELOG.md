# Changelog

All notable user-facing changes to this package are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `[messages].max_content_bytes` configuration resolving to the library's
  new-write policy (131072 UTF-8 bytes by default), including environment
  overrides and full config validation.
- Message storage delegation and explicit migration support for message/read state.

- Server CLI foundation with layered TOML/environment configuration, user/system
  runtime paths, stderr tracing, explicit SQLite migration commands, configuration
  validation, and trusted local-user provisioning.

- Enable the library's optional HS256 JWT access-token adapter for future server wiring.
