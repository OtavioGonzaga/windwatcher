# Windwatcher

API de chat **self-hosted** escrita em Rust. Sem dependências externas obrigatórias - funciona imediatamente com SQLite embutido.

## Features

- **Autenticação** com JWT + Argon2id
- **Chat em tempo real** via WebSockets
- **Salas Direct** (1:1) e de Grupo
- **Mensagens assíncronas** com fila de background jobs
- **Multi-database** - SQLite (default), PostgreSQL, MySQL, MongoDB
- **Migrations automáticas** no arranque
- **Arquitetura Hexagonal** (Ports & Adapters)

---

## Arranque Rápido

```sh
# Não é necessária nenhuma configuração - usa SQLite local automaticamente
cargo run
```

O servidor fica disponível em `http://localhost:3000`.
Um ficheiro `windwatcher_local_data.db` é criado automaticamente no directório actual.

---

## Configuração

A configuração pode ser feita por variáveis de ambiente, ficheiro `windwatcher.toml`, ou uma combinação de ambos. Variáveis de ambiente têm prioridade.

### Variáveis de Ambiente

| Variável                        | Default                                       | Descrição                                                                    |
| ------------------------------- | --------------------------------------------- | ---------------------------------------------------------------------------- |
| `WINDWATCHER_DATABASE_URL`      | `sqlite://windwatcher_local_data.db?mode=rwc` | URL de conexão à base de dados                                               |
| `WINDWATCHER_QUEUE_PROVIDER`    | `sqlite`                                      | Provider planejado da fila: `memory`, `sqlite`, `postgres`, `mysql`, `redis` |
| `WINDWATCHER_QUEUE_URL`         | `sqlite://windwatcher_jobs.db?mode=rwc`       | URL própria da fila planejada; não usa `DATABASE_URL` como fallback          |
| `WINDWATCHER_QUEUE_NAME`        | `chat_messages`                               | Nome lógico da fila planejada                                                |
| `WINDWATCHER_QUEUE_CONCURRENCY` | `4`                                           | Concorrência planejada dos workers                                           |
| `WINDWATCHER_JWT_SECRET`        | _(fraco - altere em produção)_                | Segredo para assinar tokens JWT                                              |
| `WINDWATCHER_JWT_EXPIRY_SECS`   | `86400`                                       | Validade do JWT em segundos (padrão: 24h)                                    |
| `WINDWATCHER_SERVER_HOST`       | `0.0.0.0`                                     | Endereço de bind do servidor HTTP                                            |
| `WINDWATCHER_SERVER_PORT`       | `3000`                                        | Porta do servidor HTTP                                                       |

### `windwatcher.toml` (opcional)

```toml
database_url      = "postgres://user:pass@localhost/windwatcher"
queue_provider    = "sqlite"
queue_url         = "sqlite://windwatcher_jobs.db?mode=rwc"
queue_name        = "chat_messages"
queue_concurrency = 4
jwt_secret        = "um-segredo-muito-forte-e-longo"
jwt_expiry_secs   = 3600
server_port       = 8080
```

As configurações `queue_*` pertencem à fila e são independentes de `database_*`. Mesmo que a aplicação use PostgreSQL ou MySQL como banco principal, a fila pode continuar em SQLite, Redis ou outro provider suportado. O plano detalhado para a migração para Apalis está em `.agents/memory/apalis_queue_plan.md`.

---

## Compilação por Base de Dados

O binário padrão inclui suporte a **SQLite**. Para outros backends:

```sh
# PostgreSQL
cargo build --no-default-features --features postgres

# MySQL / MariaDB
cargo build --no-default-features --features mysql

# MongoDB
cargo build --no-default-features --features mongodb

# Todos os backends SQL
cargo build --features postgres,mysql,sqlite,mongodb
```

O backend é detectado automaticamente pelo prefixo da `DATABASE_URL`:

| Prefixo                          | Backend    |
| -------------------------------- | ---------- |
| `sqlite://`                      | SQLite     |
| `postgres://` \| `postgresql://` | PostgreSQL |
| `mysql://` \| `mariadb://`       | MySQL      |
| `mongodb://` \| `mongodb+srv://` | MongoDB    |

---

## API Reference (resumo)

### Rotas principais

- `GET  /health` - Health check
- `POST /auth/register` - Criar conta
- `POST /auth/login` - Obter JWT
- `GET  /users/me` - Perfil autenticado (Bearer)
- `POST /rooms/direct` - Obter/crear sala 1:1 (Bearer)
- `POST /rooms/group` - Criar sala de grupo (Bearer)
- `POST /rooms/:room_id/messages` - Enfileira mensagem (202 + message_id)
- `GET  /rooms/:room_id/messages` - Listar mensagens (cursor UUIDv7)
- `PUT  /rooms/:room_id/read` - Marcar como lidas
- `GET  /ws?token=<jwt>` - WebSocket upgrade (JWT validado antes do upgrade)

### Exemplos rápidos

#### `POST /auth/register`

```json
{
	"username": "alice",
	"email": "alice@example.com",
	"password": "minimo8chars"
}
```

Resposta: `201 Created` com o utilizador criado.

#### `POST /auth/login`

```json
{ "email": "alice@example.com", "password": "minimo8chars" }
```

Resposta: `200 OK` com `{ "token": "<jwt>", "user": { ... } }`.

#### `POST /rooms/:room_id/messages`

```json
{ "content": "Olá a todos!" }
```

Resposta: `202 Accepted` com `{ "message_id": "<uuid>" }`.

---

## Arquitectura (resumo)

```
src/
├── main.rs              <- bootstrap e wiring
├── config.rs            <- AppConfig (figment + fallback SQLite)
├── error.rs             <- AppError centralizado com IntoResponse
├── state.rs             <- AppState injectado pelo Axum
├── domain/
│   ├── models.rs        <- User, Room, Message (sem deps de infra)
│   └── ports.rs         <- Traits: UserRepository, ChatRepository, JobQueue
├── db/
│   ├── seaorm/          <- Adapter SQL (entidades, migrations, repos)
│   └── mongodb/         <- Adapter NoSQL (feature "mongodb")
├── application/
│   ├── user_service.rs  <- registo, autenticação, JWT
│   └── chat_service.rs  <- salas, enqueue, listagem
├── api/
│   ├── http/            <- handlers Axum + extractors JWT
│   └── ws/              <- WsManager (DashMap) + handler de upgrade
└── jobs/
    └── chat_processor.rs <- InMemoryJobQueue + worker atual
```

Fluxo de mensagem (resumo):

```
POST /rooms/:id/messages
  -> extractor JWT valida o token
  -> ChatService::enqueue_message() gera UUIDv7 e devolve 202
      -> InMemoryJobQueue envia para canal tokio::mpsc
          -> worker persiste na BD
          -> incrementa contadores de não-lidas
          -> WsManager faz broadcast aos membros online
```

Planejamento da próxima etapa: substituir o worker atual por Apalis mantendo `domain::ports::JobQueue` como contrato da aplicação. O default planejado da fila é SQLite em `windwatcher_jobs.db`, separado do banco principal. RabbitMQ/AMQP não faz parte desta fase.

---

## Como contribuir / agentes

- Antes de iniciar qualquer alteração importante, leia `AGENTS.md` e `.agents/memory/architecture.md`.
- Artefactos do agente e informação persistente ficam em `.agents/`.
- Crie uma branch, adicione testes e abra um PR com descrição clara.

---

## Desenvolvimento

```sh
# Verificação rápida
cargo check

# Build e execução
cargo run

# Logs detalhados
RUST_LOG=windwatcher=debug,sea_orm=warn cargo run
```

### Pré-requisitos

- Rust 1.80+ (edition 2024)
- Para PostgreSQL/MySQL: servidor de base de dados disponível
- Para MongoDB: compilar com `--features mongodb`

---

## Licença

Este projeto está licenciado sob a licença MIT. Consulte [LICENSE](LICENSE).
