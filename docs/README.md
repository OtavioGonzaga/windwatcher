# Documentation

- [Architecture](architecture.md): current architectural foundations.
- [Identity](identity.md): current user identity domain behavior.
- [Local authentication](authentication.md): credentials, application services,
  and storage/password port contracts.
- [Sessions and tokens](sessions.md): session lifecycle and token contracts.
- [Realtime](realtime.md): session-bound WebSocket tickets, notifications, audiences,
  connection lifecycle and HTTP reconciliation.
- [HTTP API](http-api.md): v1 transport contracts, authentication, errors,
  messaging client journey, pagination/idempotency, OpenAPI/Swagger, CORS and
  peer-IP quotas (router only; no serving runtime yet).
- [Conversations](conversations.md): membership, consumer inbox queries and activity.
- [Messages](messages.md): message projections, replies, tombstones and own read state.
- [Pagination](pagination.md): opaque resource-scoped keyset continuations.
- [Persistence](persistence.md): semantic ports, SQLite snapshots and atomic commands.
- [Server configuration](server-configuration.md): operational configuration,
  paths, secrets, and logging.
- [CLI](cli.md): explicit migration and user-provisioning commands.
- [Architectural Decision Records](adr/README.md): significant decisions made
  with their implementation.
- [Contributing](../CONTRIBUTING.md): development workflow and repository rules.
