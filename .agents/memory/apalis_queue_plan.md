# Plano de Implementacao - Filas com Apalis

Ultima atualizacao: 2026-05-24 — Implementação concluída (Apalis runtime + adapters básicos).

## Situação geral

A arquitetura proposta foi implementada: a fila foi migrada do `tokio::mpsc` original para um runtime Apalis com adapters `memory`, `sqlite`, `postgres`, `mysql` e `redis` (os adapters SQL/Redis são activáveis via features). O contrato da aplicação (`domain::ports::JobQueue`) foi preservado; `ChatService` continua a depender apenas de `Arc<dyn JobQueue>`.

## O que foi implementado

- `QueueProvider` e campos `queue_*` adicionados a `AppConfig` (`src/config.rs`) com defaults apropriados (`queue_provider = "sqlite"`, `queue_url = "sqlite://windwatcher_jobs.db?mode=rwc"`, `queue_name = "chat_messages"`, `queue_concurrency = 4`).
- `src/jobs/processor.rs` criado contendo `process_chat_message` (lógica de negócio do worker).
- `src/jobs/runtime.rs` criado com `build_job_runtime` e `JobRuntime` que escolhe o adapter e inicializa o worker Apalis.
- Adapters implementados:
  - `src/jobs/memory.rs` (Apalis memory storage)
  - `src/jobs/sql.rs` (Apalis SQL storage para sqlite/postgres/mysql)
  - `src/jobs/redis.rs` (Apalis Redis storage)
- `src/main.rs` actualizado para usar `build_job_runtime(...).await?` e para injetar `job_runtime.queue()` no `ChatService`.
- `src/domain/ports.rs` e documentação atualizadas para remover referências ao antigo `InMemoryJobQueue` direto e apontar para as implementações Apalis.
- `src/jobs/chat_processor.rs` (antigo) removido; sua lógica foi consolidada em `processor.rs` e em adapters Apalis.
- `Cargo.toml` features e dependências relacionadas a Apalis / apalis-sql / apalis-redis / sqlx já presentes e configuradas.

## O que falta / follow-ups recomendados

- Revisão de warnings de documentação gerados por `cargo doc --no-deps` e limpeza de comentários não utilizados (se desejar warnings = 0).
- Cobertura de testes unitários/integrados para os adapters SQL/Redis — os testes que toquem Postgres/MySQL/Redis devem ser feature-gated e/ou `#[ignore]` por padrão.
- Atualizar `README.md`, `.env.example` e `windwatcher.example.toml` se ainda contiverem referências ao mecanismo antigo.
- Considerar adicionar um teste de integração end-to-end que inicia o runtime `queue-memory` e valida que um `ChatMessageJob` enfileirado é processado (pode ser acrescentado como parte da suite de CI em modo `queue-memory`).

## Verificações recomendadas (executadas manualmente após a mudança)

```sh
cargo fmt --check
cargo check
cargo test
cargo doc --no-deps

# Com features específicas
cargo check --no-default-features --features sqlite,queue-sqlite
cargo check --no-default-features --features postgres,queue-postgres
cargo check --no-default-features --features mysql,queue-mysql
cargo check --features queue-memory
cargo check --features queue-redis
```

## Critérios de aceite

- `ChatService` continua dependendo apenas de `JobQueue`.
- `domain/` e `application/` não importam Apalis/SQLx/Redis.
- O provider default da fila é SQLite independente do `database_url` da aplicação.
- `cargo check`, `cargo test` e `cargo doc --no-deps` devem passar sem introduzir novos warnings relevantes relacionados à integração do Apalis.
