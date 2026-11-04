# 0018: Static documentation website

- Status: Accepted
- Date: 2026-11-04

## Context

Project guides and architectural decisions live in repository Markdown, while
the HTTP contract is generated from the server's Utoipa route composition.
Publishing searchable documentation must not create another manually maintained
contract or require an operational documentation service.

## Decision

Use a separate `website/` application with Fumadocs, Next.js static export and
GitHub Pages. Keep `docs/` authoritative for prose and
`windwatcher_server::http::openapi()` authoritative for HTTP operations.
Consume Markdown at build time and export OpenAPI without starting the server.
Generate endpoint pages, search data and downloadable artifacts from those inputs;
do not commit generated content. Keep the deployment prefix centralized.

The deployed site requires only static file hosting. Search runs in the browser;
there is no API request proxy, documentation backend, external search service,
authentication or MCP server. The server's existing Swagger UI is unchanged.

## Consequences

Contributors need Node.js and pnpm only when working on the website. A dedicated
workflow validates and publishes the static output independently of Rust CI and
release workflows. Website builds compile the server's OpenAPI exporter, so
changes to either crate can require a documentation rebuild. New pages and
operations must pass generated coverage and link checks before publication.
