# Espaço de Trabalho do Agente

Este diretório serve como a base centralizada de conhecimento e ferramentas para os agentes de IA que auxiliam neste projeto. O objetivo é manter um layout organizado, consistente e agnóstico a qualquer ferramenta ou provedor de IA.

## Resumo dos Diretórios

| Diretório    | Descrição                                         | Versionado |
| :----------- | :------------------------------------------------ | :--------: |
| `memory/`    | Conhecimento persistente e estratégico do projeto |     ✅     |
| `prompts/`   | Prompts otimizados e reutilizáveis para agentes   |     ✅     |
| `artifacts/` | Arquivos de trabalho temporários (efêmeros)       |     ❌     |

---

## Detalhamento

### `memory/` (Versionado)

Armazena o conhecimento de longo prazo. Documentação que não muda frequentemente, mas é crucial para os agentes entenderem o contexto do domínio ou da arquitetura.

### `prompts/` (Versionado)

Prompts otimizados e testados para garantir que os agentes sigam o estilo e as regras do projeto ao executar tarefas específicas, promovendo consistência e eficiência.

### `artifacts/` (Não Versionado)

Espaço de trabalho temporário para todos os arquivos gerados ou utilizados durante a execução de uma tarefa. Inclui planos temporários, notas rápidas, rascunhos, logs de debug e quaisquer relatórios intermediários. Não deve ser versionado.

---

> **Nota importante:** Apenas `memory/`, `prompts/` e este arquivo (`README.md`) devem ser commitados no controle de versão (Git). A pasta `artifacts/` é efêmera e está configurada para ser ignorada.

## Como Contribuir

- **Adicionar Prompt:** Criar arquivo em `prompts/` com descrição clara no topo
- **Adicionar Knowledge:** Criar arquivo markdown em `memory/` com contexto de domínio

## Critérios de Versionamento

### ✅ Versionar (.agents/memory/ e .agents/prompts/)

- Conhecimento que é reutilizável por múltiplos agentes
- Instruções que evolem com o projeto
- Documentação que muda raramente

### ❌ Não Versionar (.agents/artifacts/)

- Relatórios de execução (outputs de tarefas)
- Logs intermediários ou debugging
- Ficheiros scratch/temporários
- Planos de tarefa específicos

**Exemplo:** Um agente faz análise de cobertura de testes -> salva relatório em `artifacts/`,
mas se descobrir um novo padrão de teste -> documenta em `memory/`.
