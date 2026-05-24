# Windwatcher API - Estado da Implementação

Última atualização: 2026-05-24 — Job runtime Apalis implementado; substituído o `InMemoryJobQueue` antigo e removido `chat_processor.rs`.

## Estado: ✅ Compilando e funcional

`cargo build` e `cargo check` passam sem erros.  
As rotas principais (registo, login, envio de mensagem via 202, WebSocket) foram validadas.

## Estrutura de ficheiros (resumo)

```
src/
├── main.rs                    <- wiring completo (usa jobs::build_job_runtime)
├── config.rs                  <- AppConfig + defaults para queue_* (queue_provider, queue_url)
├── error.rs                   <- AppError + IntoResponse
├── state.rs                   <- AppState (Clone)
├── domain/
│   ├── models.rs
│   └── ports.rs               <- UserRepository, ChatRepository, JobQueue traits
├── db/ (seaorm + optional mongodb)
├── application/
│   ├── user_service.rs
│   └── chat_service.rs        <- depende de Arc<dyn JobQueue>
├── api/                       <- HTTP handlers + WebSocket manager
└── jobs/                      <- Apalis-based job runtime
    ├── mod.rs                 <- re-exports (processor, runtime, adapters)
    ├── processor.rs          <- `process_chat_message` (business logic)
    ├── runtime.rs            <- `build_job_runtime`, `JobRuntime` abstraction
    ├── memory.rs             <- Apalis memory adapter (dev/test)
    ├── sql.rs                <- Apalis SQL adapters (sqlite/postgres/mysql)
    └── redis.rs              <- Apalis Redis adapter
```

## Rotas disponíveis

| Método | Caminho             | Auth         | Descrição                               |
| ------ | ------------------- | ------------ | --------------------------------------- |
| GET    | /health             | -            | Health check                            |
| POST   | /auth/register      | -            | Criar conta                             |
| POST   | /auth/login         | -            | Obter JWT                               |
| GET    | /users/me           | Bearer       | Perfil do utilizador                    |
| POST   | /rooms/direct       | Bearer       | Sala direct 1:1                         |
| POST   | /rooms/group        | Bearer       | Sala de grupo                           |
| POST   | /rooms/:id/messages | Bearer       | Enviar mensagem (202 + message_id)      |
| GET    | /rooms/:id/messages | Bearer       | Listar mensagens (cursor: ?before=uuid) |
| PUT    | /rooms/:id/read     | Bearer       | Marcar como lido                        |
| GET    | /ws                 | query: token | WebSocket upgrade                       |

## Decisões técnicas (resumo)

- **Fila de jobs**: Apalis com providers `memory`, `sqlite`, `postgres`, `mysql` e `redis`. O provider é escolhido em runtime via `AppConfig::queue_provider`.
- **API do domínio**: `application/` e `domain/` continuam a depender apenas do trait `JobQueue` (sem imports de Apalis/SQLx/Redis).
- **Configuração da fila**: separada do `database_url`. Defaults em `AppConfig::default()`:
  - `queue_provider = "sqlite"`
  - `queue_url = "sqlite://windwatcher_jobs.db?mode=rwc"`
  - `queue_name = "chat_messages"`
  - `queue_concurrency = 4`
- **Processamento**: `process_chat_message` foi extraído para `src/jobs/processor.rs` e é usado por todos os adapters como handler.
- **Startup**: `main.rs` chama `build_job_runtime(&config, chat_repo, ws_manager).await?` e obtém `JobRuntime::queue()` para injetar em `ChatService`.

## Configuração

Via `WINDWATCHER_*` env vars ou `windwatcher.toml`:

- `WINDWATCHER_DATABASE_URL` - qualquer URL postgres/mysql/sqlite
- `WINDWATCHER_QUEUE_PROVIDER` - provider da fila (`memory`, `sqlite`, `postgres`, `mysql`, `redis`)
- `WINDWATCHER_QUEUE_URL` - URL da fila (default `sqlite://windwatcher_jobs.db?mode=rwc`)
- `WINDWATCHER_QUEUE_NAME` - nome lógico da fila (`chat_messages`)
- `WINDWATCHER_QUEUE_CONCURRENCY` - concorrência por worker (default `4`)
- `WINDWATCHER_JWT_SECRET` - segredo JWT (obrigatório em produção!)
- `WINDWATCHER_SERVER_PORT` - default 3000
- `WINDWATCHER_JWT_EXPIRY_SECS` - default 86400
