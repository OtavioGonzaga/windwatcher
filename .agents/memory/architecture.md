# Windwatcher API - Estado da Implementação

Última atualização: implementação inicial completa e funcionando.

## Estado: ✅ Compilando e funcional

`cargo build` e `cargo check` passam sem erros.  
Smoke tests manuais confirmados: register, login (JWT), credencial inválida.

## Estrutura de ficheiros (35 arquivos)

```
src/
├── main.rs                    <- wiring completo
├── config.rs                  <- AppConfig + fallback SQLite
├── error.rs                   <- AppError + IntoResponse
├── state.rs                   <- AppState (Clone)
├── domain/
│   ├── models.rs              <- User, Room, RoomUser, Message, enums
│   └── ports.rs               <- UserRepository, ChatRepository, JobQueue traits
├── db/
│   ├── mod.rs
│   ├── seaorm/
│   │   ├── mod.rs             <- setup_database() + re-exports
│   │   ├── entities/          <- DeriveEntityModel para 4 tabelas
│   │   ├── migrations/        <- 4 migrações + Migrator
│   │   ├── user_repo.rs       <- SeaOrmUserRepository
│   │   └── chat_repo.rs       <- SeaOrmChatRepository
│   └── mongodb/               <- feature-gated (#[cfg(feature = "mongodb")])
│       ├── mod.rs             <- setup_mongodb()
│       ├── setup.rs           <- criação de índices
│       ├── user_repo.rs       <- MongoUserRepository
│       └── chat_repo.rs       <- MongoChatRepository
├── application/
│   ├── user_service.rs        <- register, authenticate (argon2+JWT), get_by_id
│   └── chat_service.rs        <- rooms, enqueue_message (-> fila), list, mark_as_read
├── api/
│   ├── mod.rs
│   ├── http/
│   │   ├── mod.rs             <- router() com TraceLayer + CorsLayer
│   │   ├── extractors.rs      <- AuthenticatedUser, AdminUser (FromRequestParts)
│   │   ├── auth.rs            <- POST /auth/register, POST /auth/login
│   │   ├── users.rs           <- GET /users/me
│   │   └── chat.rs            <- rooms + messages handlers
│   └── ws/
│       ├── mod.rs
│       ├── manager.rs         <- WsManager (DashMap + mpsc channels)
│       └── handler.rs         <- GET /ws?token=<jwt>
└── jobs/
    ├── mod.rs
    └── chat_processor.rs      <- InMemoryJobQueue + process_chat_message + start_worker
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

## Decisões técnicas

- **Fila de jobs**: Apalis com adapters `memory`, `sqlite`, `postgres`, `mysql` e `redis`; RabbitMQ/AMQP fora do ciclo atual.
- **Configuração da fila**: Separada da configuração do banco; Configuração padrão: `queue_provider = "sqlite"` e `queue_url = "sqlite://windwatcher_jobs.db?mode=rwc"`.
- **Banco padrão**: SQLite local (`windwatcher_local_data.db`) sem configuração
- **Migrations**: automáticas no arranque via `Migrator::up()`
- **JWT**: HS256, expiração configurável (`jwt_expiry_secs`)
- **Passwords**: Argon2id
- **IDs**: UUIDv7 (ordenável temporalmente, funciona como cursor)
- **WebSocket**: upgrade via query param `?token=<jwt>`

## Configuração

Via `WINDWATCHER_*` env vars ou `windwatcher.toml`:

- `WINDWATCHER_DATABASE_URL` - qualquer URL postgres/mysql/sqlite
- `WINDWATCHER_QUEUE_PROVIDER` - provider da fila planejada (`memory`, `sqlite`, `postgres`, `mysql`, `redis`)
- `WINDWATCHER_QUEUE_URL` - URL propria da fila planejada; default SQLite separado (`sqlite://windwatcher_jobs.db?mode=rwc`)
- `WINDWATCHER_QUEUE_NAME` - nome logico da fila planejada; default `chat_messages`
- `WINDWATCHER_QUEUE_CONCURRENCY` - concorrencia dos workers planejados; default `4`
- `WINDWATCHER_JWT_SECRET` - segredo JWT (obrigatório em produção!)
- `WINDWATCHER_SERVER_PORT` - default 3000
- `WINDWATCHER_JWT_EXPIRY_SECS` - default 86400
