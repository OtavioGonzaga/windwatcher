# Diretrizes do Repositório

## Estado atual da implementação (resumo)

Última atualização: 2026-05-19 - Implementação inicial completa e funcional.

- `cargo check` / `cargo build` passam sem erros; smoke tests básicos (registo, login, JWT) validados.
- Banco de dados padrão: SQLite local (`windwatcher_local_data.db`) criado automaticamente.
- Fila de background: implementação atualmente em memória (tokio mpsc). Planeado migrar para `apalis` para jobs persistentes.
- Migrations: executadas automaticamente no arranque via `Migrator::up()` (SeaORM).

Para o estado detalhado da implementação (lista completa de rotas, decisões técnicas e TODOs) consulte: `.agents/memory/architecture.md` - este é o estado canónico que todos os agentes devem ler antes de qualquer alteração.

## Artefatos do Agente

Use `AGENTS.md` como o ponto de entrada de instruções compartilhado para todos os agentes de codificação de IA. Mantenha os arquivos do agente em `.agents/` e evite diretórios específicos de ferramentas/fornecedores.

- Conhecimento compartilhado persistente (versionado) pertence a `.agents/memory/`.
- Prompts reutilizáveis (versionados) pertencem a `.agents/prompts/`.
- Artefatos temporários (não versionados) pertencem a `.agents/artifacts/`.
- Não crie artefatos markdown na raiz do repositório, a menos que explicitamente solicitado.

---

## Visão Geral do Projeto

**Windwatcher** é uma API de chat self-hosted escrita em Rust.  
Stack: Axum · Tokio · SeaORM · Argon2 · JWT · WebSockets · DashMap.

Leia `.agents/memory/architecture.md` para o estado atual da implementação, lista completa de rotas e decisões técnicas registadas.

---

## Arquitetura

O projeto segue **Ports & Adapters (Hexagonal)** com separação estrita entre camadas:

```
Transporte (api/)  ->  Aplicação (application/)  ->  Domínio (domain/)  <-  Adapters (db/)
```

### Camadas e responsabilidades

| Módulo             | Responsabilidade                            | Regra                        |
| ------------------ | ------------------------------------------- | ---------------------------- |
| `domain/models.rs` | Structs agnósticos de BD                    | Sem imports de infra         |
| `domain/ports.rs`  | Traits (interfaces) dos repositórios e fila | Sem imports de infra         |
| `application/`     | Orquestração de casos de uso                | Depende apenas de `domain/`  |
| `db/seaorm/`       | Adapter SQL (Postgres · MySQL · SQLite)     | Implementa ports             |
| `db/mongodb/`      | Adapter NoSQL (feature-gated)               | Implementa ports             |
| `api/http/`        | Handlers Axum, extractors JWT               | Delega tudo à `application/` |
| `api/ws/`          | WebSocket upgrade + WsManager em memória    | Sem lógica de negócio        |
| `jobs/`            | Worker in-memory + `InMemoryJobQueue`       | Implementa `JobQueue` port   |
| `state.rs`         | `AppState` (Clone) injetado pelo Axum       | Apenas composição            |
| `config.rs`        | `AppConfig` via figment + fallback SQLite   | Lida no main                 |
| `error.rs`         | `AppError` + `IntoResponse`                 | Centralizado                 |

### Fluxo de uma mensagem de chat

```
POST /rooms/:id/messages
  -> auth extractor (JWT)
  -> ChatService::enqueue_message()          <- gera UUIDv7, retorna 202
      -> InMemoryJobQueue::enqueue()
          -> Worker tokio::spawn
              -> chat_repo.add_message()
              -> chat_repo.increment_unread()
              -> WsManager::send_to_users()  <- broadcast aos membros online
```

---

## Convenções de Código

- **Rust Edition 2024** - use as novas features quando adequado.
- Todos os IDs são `uuid::Uuid` gerados com `Uuid::now_v7()` (UUIDv7).
- Timestamps são `chrono::DateTime<Utc>` no domínio; `DateTimeWithTimeZone` nas entidades SeaORM.
- Erros retornam `Result<_, AppError>` em todos os serviços e repositórios.
- Handlers Axum retornam `Result<_, AppError>` - o `IntoResponse` cuida do mapeamento HTTP.
- Nunca coloque lógica de negócio em handlers; delege ao service correspondente.
- Nunca coloque imports de Axum/HTTP em `domain/` ou `application/`.

---

## Como Adicionar uma Feature

### Novo endpoint HTTP

1. Crie ou edite um handler em `src/api/http/<módulo>.rs`.
2. Registe a rota em `src/api/http/mod.rs` -> `router()`.
3. Se precisar de lógica nova, adicione ao service em `src/application/`.
4. Se o service precisar de novo acesso a dados, adicione o método ao trait em `src/domain/ports.rs` e implemente em **ambos** os adapters (`db/seaorm/` e `db/mongodb/`).

### Novo modelo de domínio

1. Declare a struct em `src/domain/models.rs`.
2. Crie a entidade SeaORM em `src/db/seaorm/entities/`.
3. Crie a migration em `src/db/seaorm/migrations/` e registe-a no `Migrator`.
4. Adicione os métodos necessários ao trait em `src/domain/ports.rs`.
5. Implemente nos dois adapters.

### Novo background job

1. Defina a struct do job em `src/domain/ports.rs` ou em `src/jobs/`.
2. Adicione o método ao trait `JobQueue`.
3. Implemente em `InMemoryJobQueue`.
4. Crie a função de processamento em `src/jobs/`.
5. Registe o worker em `main.rs`.

---

## Comandos Úteis

```sh
# Verificação rápida (sem linkar)
cargo check

# Build debug
cargo build

# Build release
cargo build --release

# Com suporte a MongoDB
cargo build --features mongodb

# Com suporte a Postgres (sem SQLite)
cargo build --no-default-features --features postgres

# Correr localmente (SQLite automático, porta 3000)
cargo run

# Variáveis de ambiente mais comuns
WINDWATCHER_DATABASE_URL=postgres://user:pass@localhost/windwatcher
WINDWATCHER_JWT_SECRET=segredo-forte-aqui
WINDWATCHER_SERVER_PORT=8080
RUST_LOG=windwatcher=debug,sea_orm=warn
```

---

## Ficheiros de Memória do Agente

| Ficheiro                                   | Conteúdo                              |
| ------------------------------------------ | ------------------------------------- |
| `.agents/memory/architecture.md`           | Estado da implementação, rotas, TODOs |
| `.agents/artifacts/implementation_plan.md` | Blueprint técnico original            |
| `.agents/artifacts/steps.md`               | Plano de passos por fase              |

**Antes de qualquer tarefa de código**, leia `.agents/memory/architecture.md` para ter o estado actual do projeto.
**Depois de concluir** alterações estruturais, actualize `.agents/memory/architecture.md`.

---

## Pendente / TODOs (resumo)

- [ ] Substituir `InMemoryJobQueue` por `apalis` para persistência de jobs.
- [ ] Testes de integração.
- [ ] Rate limiting (tower middleware).
- [ ] Refresh tokens.
- [ ] Verificar membros de sala antes de permitir envio de mensagem.
- [ ] Endpoints admin (listar utilizadores, etc.).

---

## Notas finais

Siga estritamente as convenções descritas acima. Se tiver dúvidas sobre o estado do código ou decisões técnicas, consulte `.agents/memory/architecture.md` antes de editar o código.
