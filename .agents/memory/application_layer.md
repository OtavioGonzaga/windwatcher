# Application Layer - Status & Decisões

**Camada:** `src/application/`

## Arquivos criados

| Arquivo                           | Conteúdo                                   |
| --------------------------------- | ------------------------------------------ |
| `src/application/mod.rs`          | Re-exporta `chat_service` e `user_service` |
| `src/application/user_service.rs` | `UserService`, DTOs de auth, `Claims` JWT  |
| `src/application/chat_service.rs` | `ChatService`, DTOs de chat                |

## Decisões relevantes

- **Argon2id** via `Argon2::default()` (algoritmo padrão da crate `argon2 0.5`).
- **JWT HS256** via `jsonwebtoken 9` com `Header::default()`.  
  Conversão de erro via `From<jsonwebtoken::errors::Error> for AppError` já existente em `error.rs`.
- `decode_token` usa `Validation::default()` - valida exp, iat e assinatura automaticamente.
- `enqueue_message` gera o `Uuid::now_v7()` **antes** de enfileirar, retornando-o ao chamador como cursor otimista.
- Limite de mensagens: padrão 50, teto 100 (`dto.limit.unwrap_or(50).min(100)`).
- Validações de negócio (username vazio, `@` no email, senha curta, conteúdo vazio) lançam `AppError::Validation`.
- Email duplicado lança `AppError::Conflict`.
- Credenciais inválidas sempre lançam `AppError::Unauthorized("invalid credentials")` - sem revelar se o e-mail existe.

## Estado do build

`cargo check` - 0 erros, 28 warnings esperados de `dead_code` (camadas superiores ainda não implementadas).
