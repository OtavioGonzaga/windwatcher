# Changelog

All notable user-facing changes to this package are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Local registration and password authentication through asynchronous identity
  storage and password hashing ports, with explicit self-registration policy.
- Zeroized password secrets, opaque PHC hashes, and an Argon2id adapter.
- Persisted enable/disable use cases for trusted operator callers.
- Strongly typed UUIDv7 identifiers for core domain entities.
- Initial user identity model with canonical validated usernames and enable/disable behavior.
