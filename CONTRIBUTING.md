# Contributing

## Development requirements

Use the repository-managed Rust toolchain defined in
[`rust-toolchain.toml`](rust-toolchain.toml). Do not select arbitrary Rust
versions for repository work.

## Repository structure

- `crates/windwatcher`: reusable library.
- `crates/windwatcher-server`: official standalone server.
- `docs`: conceptual and operational documentation.

See [the architecture guide](docs/architecture.md) for the rationale behind
these boundaries.

## Development workflow

Identify the affected package before making a change. Keep changes focused,
update relevant documentation, and validate them locally before opening a pull
request.

## Coding standards

- Write code and documentation in English.
- Use `rustfmt` for formatting.
- Prefer idiomatic, explicit Rust; do not suppress warnings without
  justification.
- Add public APIs deliberately. Public items need appropriate rustdoc, while
  implementation details should remain private where possible.
- Prefer small, clear abstractions over speculative generalization.
- Do not add placeholder modules, features, or extension points for roadmap
  items.
- Respect package boundaries and the server-to-library dependency direction.

## Local validation

Run the following commands before finishing a change:

```bash
cargo fmt --all -- --check

cargo check \
  --workspace \
  --all-targets

cargo clippy \
  --workspace \
  --all-targets \
  --all-features \
  -- \
  -D warnings

cargo test \
  --workspace \
  --all-targets \
  --all-features

cargo doc \
  --workspace \
  --no-deps
```

Future CI must reuse these expectations.

## Testing

New behavior should be tested, and bug fixes should include regression tests
when reasonable. Do not add artificial tests for trivial implementation details
or documentation-only changes. Future adapters should share contract-test suites
where appropriate.

## SQLite migrations

Primary SQLite migrations live in `crates/windwatcher/migrations/sqlite`; queue
SQLite migrations live independently in `crates/windwatcher/migrations/queue/sqlite`.
Add stable, sequential forward-only SQL files; never edit a released migration.
SQL files use LF line endings because migration checksums are platform-independent.
SQLite integration tests create disposable temporary databases and require no
external database server or `DATABASE_URL`.

## Documentation

Keep the README concise. Put conceptual and operational documentation under
`docs/`, public library API documentation in rustdoc, and future HTTP contracts
in OpenAPI. Update relevant documentation whenever a change affects documented
behavior or contracts; document behavior and rationale rather than source-file
structure.

## Architectural decisions

Significant architectural decisions should introduce an ADR in the same change
that implements the decision. ADRs must document concrete decisions, not
anticipated roadmap architecture. Routine refactors and local implementation
details do not automatically require ADRs.

## Public API changes

Adding `pub` is an API decision. Do not make modules public merely to bypass
internal boundaries. Breaking public API changes follow Semantic Versioning for
the affected package. Prefer stable domain or application concepts over
implementation types, and do not leak server-specific types into the reusable
library without strong justification.

## Dependencies

Add dependencies only when they provide clear value, and minimize enabled
features. Do not move server or runtime framework dependencies into the library
unnecessarily. Dependencies that impose a major runtime or framework choice on
the library need deliberate justification. Prefer maintainable, well-supported
dependencies, and do not add dependencies for features that do not exist.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/). Common
prefixes include `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`,
`ci:`, and `build:`. Scopes are optional and should improve clarity.

## Changelog

Notable user-visible changes belong in the changelog of the affected package and
accumulate under `Unreleased`. Internal refactors without user-visible impact do
not need changelog noise. Changelogs follow Keep a Changelog, package versions
follow Semantic Versioning, and packages are independently versioned.

## Pull requests

Pull requests should describe the problem and change clearly, include validation
performed, update relevant documentation, add an ADR for significant
architectural decisions, and update the affected package changelog when
warranted.
