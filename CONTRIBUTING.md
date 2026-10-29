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
  --all-targets \
  --locked

cargo clippy \
  --workspace \
  --all-targets \
  --all-features \
  --locked \
  -- \
  -D warnings

cargo test \
  --workspace \
  --all-targets \
  --all-features \
  --locked

cargo test \
  --workspace \
  --doc \
  --all-features \
  --locked

RUSTDOCFLAGS="-D warnings" \
cargo doc \
  --workspace \
  --all-features \
  --no-deps \
  --locked
```

### Continuous integration

The `quality` job in [CI](.github/workflows/ci.yml) runs this baseline, including
SQLite contracts without external services. Independent jobs also validate:

- Library feature isolation: no features and each public feature individually;
  the baseline covers all features together.
- Production-target MSRV on Rust 1.99.0 (without test/dev-only targets).
- RustSec vulnerabilities and informational warnings (`cargo audit --deny warnings`),
  plus licenses, sources and dependency bans
  (`cargo deny --all-features --locked check licenses sources bans`), using
  [the reviewed dependency policy](deny.toml).
- Six parallel external adapter contracts: PostgreSQL 18 storage/queue,
  MySQL 8.4 storage/queue, MongoDB 8.2 replica-set storage and Redis 7 queue.
- Standalone server network composition: queue migrations/jobs and primary
  storage doctor CLI tests against all four services.

All checks run for every PR to `main` and push to `main`, including documentation
changes. Contributors must run the baseline locally; start disposable external
services and run the relevant real-adapter tests when changing an adapter, not
before every unrelated change. See [storage testing](docs/persistence.md),
[queue adapter tests](docs/jobs.md#real-adapter-tests) and
[operations](docs/operations.md) for detailed commands and safety requirements.
CI runs every official external adapter on every PR.

The final `CI / required` gate succeeds only when every job succeeds. After the
workflow is merged and has run, maintainers should configure that single check
as required for `main` in repository branch protection/rulesets. Repository files
do not configure those GitHub settings.

Dependabot checks Cargo and SHA-pinned GitHub Actions weekly. Cargo minor/patch
updates are grouped; major updates remain separate. There is no automatic merge,
publication, release automation or deployment in this workflow.

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

PostgreSQL and MySQL queue migrations live under
`crates/windwatcher/migrations/queue/{postgres,mysql}` and follow the same
forward-only/checksum discipline. Redis has no schema migrations. Real network
adapter tests are explicitly ignored by default; execute them against disposable
services using the commands in [durable jobs](docs/jobs.md#real-adapter-tests)
before completing queue-adapter changes.

MongoDB primary migrations live under `crates/windwatcher/migrations/mongodb` as
ordered command arrays. Each version owns its executable commands and checksum;
never edit released migrations. DDL steps must be restartable after interruption
or concurrent execution. Changes to this adapter must also run the ignored real
replica-set contracts and concurrency/query regressions against a disposable
service:

```bash
WINDWATCHER_TEST_MONGODB_URL='mongodb://127.0.0.1:27018/?replicaSet=rs0' \
  cargo test -p windwatcher --no-default-features --features mongodb --locked -- --ignored --test-threads=1
```

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
