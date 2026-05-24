# Jobs Layer - Status & Decisões

**Camada:** `src/jobs/`

## Arquivos existentes

| Arquivo                 | Conteúdo / Responsabilidade                                                 |
| ----------------------- | --------------------------------------------------------------------------- |
| `src/jobs/mod.rs`       | Re-exporta `processor`, `runtime` e os adapters (`memory`, `sql`, `redis`)  |
| `src/jobs/processor.rs` | `process_chat_message` — lógica de negócio do processamento de mensagens    |
| `src/jobs/runtime.rs`   | `JobRuntime`, `build_job_runtime` — escolhe provider e inicia worker Apalis |
| `src/jobs/memory.rs`    | Apalis memory storage adapter (dev/test)                                    |
| `src/jobs/sql.rs`       | Apalis SQL adapters para sqlite/postgres/mysql                              |
| `src/jobs/redis.rs`     | Apalis Redis adapter                                                        |

## Observações importantes

- A implementação atual substituiu o antigo `InMemoryJobQueue` baseado em `tokio::mpsc` por uma runtime Apalis. O arquivo `src/jobs/chat_processor.rs` foi removido e a lógica extraída para `processor.rs`.
- O contrato da aplicação não mudou: `ChatService` continua a depender apenas de `Arc<dyn crate::domain::ports::JobQueue>` e chama `enqueue_chat_message(...)`.
- Cada adapter expõe um `Arc<dyn JobQueue>` que implementa `enqueue_chat_message` e também inicializa o worker Apalis usando `WorkerBuilder::build_fn(...)` apontando para `process_chat_message`.

## Design / Decisões

- `processor.rs` contém a sequência: construir `Message` -> `chat_repo.add_message` -> `chat_repo.increment_unread` -> `chat_repo.get_room_members` -> `ws_manager.send_to_users`.
- `runtime.rs` centraliza a escolha do provider (`QueueProvider`) e devolve `(Arc<dyn JobQueue>, JoinHandle<()>)` (encapsulado em `JobRuntime`).
- Adapters `memory`, `sql` e `redis` apenas preenchem a interface `JobQueue` e configuram o worker; a lógica de negócio fica em `processor.rs`.
- Erros de enfileiramento são mapeados para `AppError::Internal` com mensagens descritivas.

## Wiring no startup (exemplo atual)

```rust
let ws_manager = Arc::new(WsManager::new());
let job_runtime = jobs::build_job_runtime(&config, Arc::clone(&chat_repo), Arc::clone(&ws_manager)).await?;
let job_queue: Arc<dyn ports::JobQueue> = job_runtime.queue();
let chat_service = Arc::new(ChatService::new(Arc::clone(&chat_repo), Arc::clone(&job_queue)));
```

## Estado do build

`cargo check` - 0 erros. Warnings residuais podem aparecer dependendo das features activadas. Documentação e `cargo doc --no-deps` foram mantidas; revisar warnings específicos caso queira limpeza adicional de doc warnings.
