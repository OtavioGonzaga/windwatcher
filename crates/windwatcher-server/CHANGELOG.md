# Changelog

All notable user-facing changes to this package are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Server CLI foundation with layered TOML/environment configuration, user/system
  runtime paths, stderr tracing, explicit SQLite migration commands, configuration
  validation, and trusted local-user provisioning.

- Enable the library's optional HS256 JWT access-token adapter for future server wiring.
