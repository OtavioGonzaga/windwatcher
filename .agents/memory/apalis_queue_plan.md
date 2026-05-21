# Plano de Implementacao - Filas com Apalis

Ultima atualizacao: 2026-05-20.

## Decisoes fechadas

- Usar Apalis para substituir a fila atual baseada em `tokio::mpsc`.
- Manter `domain::ports::JobQueue` como porta da aplicacao. A camada `application/` nao deve importar Apalis.
- Implementar adapters para `memory`, `sqlite`, `postgres`, `mysql` e `redis`.
- Deixar RabbitMQ/AMQP fora deste ciclo. Ele pode ser implementado depois como mais um adapter.
- Usar `sqlite` como provider default da fila.
- Nao misturar configuracao da fila com configuracao do banco principal.
- Nao usar `database_url` como fallback para `queue_url`.
- Criar uma URL default propria para fila SQLite: `sqlite://windwatcher_jobs.db?mode=rwc`.
- Usar nomes de variaveis com prefixo `WINDWATCHER_QUEUE_*`.
- Atualizar sempre `README.md`, `.env.example`, `windwatcher.example.toml` e `.agents/memory/architecture.md` ao concluir a implementacao.

## Contrato esperado

O codigo de aplicacao deve continuar dependendo apenas de:

```rust
Arc<dyn crate::domain::ports::JobQueue>
```

O metodo existente permanece:

```rust
async fn enqueue_chat_message(&self, job: ChatMessageJob) -> Result<(), AppError>;
```

`ChatMessageJob` continua sendo o payload persistido/processado pela fila.

## Configuracao alvo

Adicionar a `AppConfig`:

```rust
pub queue_provider: QueueProvider,
pub queue_url: String,
pub queue_name: String,
pub queue_concurrency: usize,
```

Adicionar enum em `src/config.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QueueProvider {
    Memory,
    #[default]
    Sqlite,
    Postgres,
    Mysql,
    Redis,
}
```

Defaults:

```text
queue_provider = "sqlite"
queue_url = "sqlite://windwatcher_jobs.db?mode=rwc"
queue_name = "chat_messages"
queue_concurrency = 4
```

Variaveis de ambiente:

```sh
WINDWATCHER_QUEUE_PROVIDER=sqlite
WINDWATCHER_QUEUE_URL=sqlite://windwatcher_jobs.db?mode=rwc
WINDWATCHER_QUEUE_NAME=chat_messages
WINDWATCHER_QUEUE_CONCURRENCY=4
```

Notas:

- `queue_url` e obrigatoria semanticamente para providers externos, mas tem default proprio para `sqlite`.
- Nao derivar `queue_provider` de `database_provider`.
- Nao copiar `database_url` para `queue_url`.
- Se `queue_provider` nao estiver habilitado por feature Cargo, a aplicacao deve falhar no boot com mensagem clara.

## Features Cargo alvo

Usar Apalis estavel `0.7.x` salvo se houver motivo tecnico para mudar.

Proposta:

```toml
[features]
default = ["sqlite", "queue-sqlite"]
postgres = ["sea-orm/sqlx-postgres"]
mysql = ["sea-orm/sqlx-mysql"]
sqlite = ["sea-orm/sqlx-sqlite"]
mongodb = ["dep:mongodb"]

queue-memory = ["dep:apalis"]
queue-sqlite = ["dep:apalis", "dep:apalis-sql", "dep:sqlx", "sqlx/sqlite"]
queue-postgres = ["dep:apalis", "dep:apalis-sql", "dep:sqlx", "sqlx/postgres"]
queue-mysql = ["dep:apalis", "dep:apalis-sql", "dep:sqlx", "sqlx/mysql"]
queue-redis = ["dep:apalis", "dep:apalis-redis"]
```

Dependencias esperadas:

```toml
apalis = { version = "0.7", optional = true, features = ["limit"] }
apalis-sql = { version = "0.7", optional = true, features = ["tokio"] }
apalis-redis = { version = "0.7", optional = true }
sqlx = { version = "0.8", optional = true, default-features = false, features = ["runtime-tokio-rustls", "chrono", "uuid", "json"] }
```

As dependências devem ser instaladas com `cargo add` para que sejam instaladas na versão estável mais recente disponível.
O agente implementador deve confirmar os nomes exatos de features de `apalis-sql` antes de fechar o patch, porque a API do crate pode mudar entre releases.

## Estrutura de arquivos alvo

Refatorar `src/jobs/` para:

```text
src/jobs/
├── mod.rs
├── processor.rs       # process_chat_message e handler comum
├── runtime.rs         # JobRuntime, build_job_runtime(...)
├── memory.rs          # adapter Apalis memory
├── sql.rs             # adapters sqlite/postgres/mysql
└── redis.rs           # adapter Redis
```

Responsabilidades:

- `processor.rs`: contem a logica hoje em `process_chat_message`.
- `runtime.rs`: escolhe adapter via `AppConfig`, retorna `Arc<dyn JobQueue>` e inicia worker Apalis.
- `memory.rs`: provider nao persistente para dev/testes.
- `sql.rs`: cria storage Apalis SQL para SQLite/Postgres/MySQL usando `queue_url`.
- `redis.rs`: cria storage Apalis Redis usando `queue_url`.

Nao colocar logica de negocio nos adapters. Adapters apenas enfileiram, fazem wiring do worker e chamam o processador comum.

## Ordem de implementacao para agente menor

1. Ler `AGENTS.md`, `.agents/memory/architecture.md` e este arquivo.
2. Adicionar `QueueProvider` e campos `queue_*` em `src/config.rs`.
3. Adicionar testes unitarios de config para defaults e override via env/TOML se ja houver padrao local para isso.
4. Atualizar `Cargo.toml` com features e dependencias opcionais.
5. Mover `process_chat_message` para `src/jobs/processor.rs`.
6. Criar `src/jobs/runtime.rs` com uma API simples:

```rust
pub struct JobRuntime {
    pub queue: Arc<dyn JobQueue>,
}

impl JobRuntime {
    pub fn queue(&self) -> Arc<dyn JobQueue>;
}

pub async fn build_job_runtime(
    config: &AppConfig,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) -> Result<JobRuntime, AppError>;
```

7. Implementar primeiro `queue-memory` para provar o desenho.
8. Implementar `queue-sqlite`.
9. Implementar `queue-postgres` e `queue-mysql` no mesmo modulo SQL.
10. Implementar `queue-redis`.
11. Trocar o wiring em `src/main.rs` para usar `build_job_runtime`.
12. Remover ou isolar `InMemoryJobQueue` antigo baseado em `tokio::mpsc`.
13. Atualizar `README.md`, `.env.example`, `windwatcher.example.toml` e `.agents/memory/architecture.md`.
14. Rodar verificacoes.

## Verificacoes obrigatorias

Minimo:

```sh
cargo fmt --check
cargo check
cargo test
cargo doc --no-deps
```

Feature checks:

```sh
cargo check --no-default-features --features sqlite,queue-sqlite
cargo check --no-default-features --features postgres,queue-postgres
cargo check --no-default-features --features mysql,queue-mysql
cargo check --features queue-memory
cargo check --features queue-redis
```

Testes que exigem servicos externos (`redis`, `postgres`, `mysql`) devem ser feature-gated e/ou `#[ignore]`, com instrucoes claras de ambiente.

## Criterios de aceite

- `ChatService` continua dependendo apenas de `JobQueue`.
- `domain/` e `application/` continuam sem imports de Apalis, SQLx, Redis ou HTTP.
- SQLite e o default da fila, usando `windwatcher_jobs.db`, separado do banco da aplicacao.
- `database_url` nunca e usado como fallback de `queue_url`.
- RabbitMQ/AMQP nao entra neste ciclo.
- README e exemplos documentam as novas configuracoes.
- `cargo check`, `cargo test` e `cargo doc --no-deps` passam sem warnings novos.
