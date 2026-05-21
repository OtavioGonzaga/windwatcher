# Jobs Layer - Status & Decisões

**Camada:** `src/jobs/`

## Arquivos criados

| Arquivo                      | Conteúdo                                                         |
| ---------------------------- | ---------------------------------------------------------------- |
| `src/jobs/mod.rs`            | Re-exporta `chat_processor`, `InMemoryJobQueue` e `start_worker` |
| `src/jobs/chat_processor.rs` | `InMemoryJobQueue`, `process_chat_message`, `start_worker`       |

## Arquivos de stub criados

| Arquivo                 | Motivo                                                                |
| ----------------------- | --------------------------------------------------------------------- |
| `src/api/mod.rs`        | Resolver `mod api` em `main.rs`                                       |
| `src/api/ws/mod.rs`     | Resolver `crate::api::ws`                                             |
| `src/api/ws/manager.rs` | **STUB** - contém `WsManager` + `SocketMessage` mínimos para compilar |

> ⚠️ O agente responsável pelo WebSocket deve **substituir** `src/api/ws/manager.rs`
> mantendo a surface pública: `WsManager::send_to_users(&[Uuid], SocketMessage)` e
> `SocketMessage::NewMessage(Message)`.

## Decisões relevantes

### InMemoryJobQueue

- Wrapper sobre `mpsc::Sender<ChatMessageJob>`.
- Implementa o port `JobQueue` via `async_trait`.
- Erro de envio (canal fechado) -> `AppError::Internal`.

### process_chat_message

- `room_id` e `sender_id` são copiados **antes** de mover `job.content` para o
  `Message`, evitando "use after partial move" (ambos são `Uuid: Copy`).
- Fluxo: persistir -> incrementar unread -> obter membros -> broadcast WS.
- Broadcast é **best-effort**: `WsManager::send_to_users` ignora erros de envio
  silenciosamente (usuários offline simplesmente não recebem a mensagem).

### start_worker

- Uma única goroutine consome o `mpsc::Receiver` em loop.
- Cada job é delegado a um `tokio::spawn` próprio: um job lento ou com falha não
  bloqueia a fila.
- Encerramento do canal logado em `tracing::warn` para facilitar diagnóstico.

### Capacidade do canal

- Definida pelo chamador na criação do par `mpsc::channel(n)`.
- Recomendação: 1024 em produção; para testes unitários, 16 é suficiente.

## Wiring no startup (exemplo)

```rust
let (tx, rx) = tokio::sync::mpsc::channel::<ChatMessageJob>(1024);
let job_queue = Arc::new(InMemoryJobQueue::new(tx));
let ws_manager = Arc::new(WsManager::new());
jobs::start_worker(rx, Arc::clone(&chat_repo), Arc::clone(&ws_manager));
```

## Estado do build

`cargo check` - 0 erros, ~65 warnings de `dead_code` esperados (camadas de API ainda
não implementadas).
