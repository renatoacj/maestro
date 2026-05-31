# Maestro — Design Spec

**Data:** 2026-05-30
**Status:** Aprovado para v1
**Codinome:** Maestro (rege seus jobs de background)

## Problema

Quem roda múltiplos serviços, timers e daemons em background (`systemd --user`, cron,
Docker, etc.) não tem visibilidade do que está ativo, do que falhou, de quando cada
coisa roda de novo, nem de quanto recurso consome. Hoje isso é gerenciado na unha, com
cheat sheets de `systemctl` e `journalctl`. Não existe um lugar único, bonito e ao vivo
para **ver e controlar** tudo.

Cenário típico: um conjunto de user-services e timers systemd — daemons, geradores,
relatórios agendados — controlados manualmente via `systemctl --user`, sem visão
unificada de estado, falhas ou consumo.

## Visão

Um **cockpit cross-platform** para todos os jobs de background da máquina —
"btop + gerenciador de systemd + Docker Desktop" num app só, leve, com design nível
Linear/cinematográfico, dark-first.

O valor central (nesta ordem de prioridade):

1. **Controlar tudo sem CLI** — ligar/desligar/reiniciar/disparar qualquer job num clique.
2. **Observabilidade de recursos** — CPU e memória por job/processo, ao vivo.
3. **Estado num olhar** — o que está ativo, próximo run, último run, saúde geral.
4. **Avisos de falha** — notificação nativa quando um job quebra ou some (secundário, mas no v1).

## Princípio arquitetural

Uma única abstração — a trait `JobProvider` — esconde a diferença entre os mecanismos
de cada OS. Todo o app conversa apenas com essa trait. Adicionar um OS/mecanismo novo é
escrever **um** provider, sem tocar no núcleo. É isso que torna o "universal
cross-platform" um incremento, não uma reescrita.

```rust
trait JobProvider {
    fn id(&self) -> ProviderId;
    fn available(&self) -> bool;                       // o mecanismo existe nesta máquina?
    async fn list(&self) -> Result<Vec<Job>>;          // descobre os jobs
    async fn status(&self, id: &JobId) -> Result<JobStatus>;
    async fn control(&self, id: &JobId, action: Action) -> Result<()>;
    async fn metrics(&self, id: &JobId) -> Result<Resources>;
    fn logs(&self, id: &JobId, follow: bool) -> LogStream;
    fn watch(&self) -> EventStream;                    // mudanças de estado empurradas
}
```

### Stack (escolha técnica, não popularidade)

- **Backend:** Rust + **Tauri v2**. Binário pequeno, consumo mínimo de RAM/CPU — um
  monitor de recursos não pode ser glutão de recursos. Acesso de sistema real e seguro.
- **D-Bus:** `zbus` (D-Bus puro em Rust, async) — fala com o systemd via session bus,
  sem root, sem shellar `systemctl`.
- **Docker:** `bollard` (cliente Docker em Rust) sobre o socket Unix.
- **Métricas:** `sysinfo` + leitura direta de cgroup v2 (`/sys/fs/cgroup/...`) para
  atribuir CPU/memória por unit/container.
- **Async:** `tokio`. **Serialização:** `serde`.
- **Frontend:** **Svelte 5** + TypeScript + SvelteKit (adapter-static, modo SPA).
  Leve, reativo, animações calmas via transitions nativas. CSS artesanal com design
  tokens (dark-first) para a UI cinematográfica sob medida — utilitários (Tailwind)
  não agregam numa interface bespoke e foram dispensados no v1.

## Componentes (Rust)

Cada peça é uma unidade isolada, com uma responsabilidade, testável com um `FakeProvider`
(sem precisar de systemd real para a lógica central).

- **`providers/`** — cada provider isolado, mesma trait.
  - `SystemdUserProvider` (v1) — session D-Bus, units `*.service` e `*.timer`.
  - `DockerProvider` (v1, estruturado; inerte sem Docker instalado).
  - Depois: `SystemdSystemProvider` (v2, via polkit), `CronProvider` (v2),
    `LaunchdProvider` (v3, macOS), `WindowsProvider` (v3, Task Scheduler + Services).
- **`core/registry`** — descobre providers disponíveis e agrega numa lista unificada.
- **`core/sampler`** — `MetricsSampler`: loop throttled (~1–2s) que amostra recursos só
  dos jobs ativos e emite deltas. Erro em um job é isolado e não derruba o loop.
- **`core/logstreamer`** — tail de journald (sd-journal) / docker logs / arquivos, com
  backpressure.
- **`core/watcher`** — `StateWatcher`: assina sinais do D-Bus (systemd) e eventos do
  Docker; emite mudanças de estado.
- **`core/eventbus`** — distribui eventos para a UI e para o Notifier.
- **`core/notifier`** — notificação nativa em falha/sumiço de job.
- **`ipc/`** — comandos Tauri tipados; camada fina sobre o núcleo. Único ponto que o
  frontend pode chamar. Sem exec arbitrário exposto.

## Modelo de dados

```
Job {
  id          // qualificado: "systemd-user:my-worker.service"
  provider    // systemd-user | docker | cron | launchd | windows
  kind        // service (contínuo) | scheduled (timer/cron) | container
  name, description, command
  state       // active | inactive | failed | activating | …
  enabled     // sobe no boot? (autostart)
  schedule?   // { next_run, last_run }   — só para scheduled
  resources   // { cpu_pct, mem_bytes, pids[] }
  health      // ok | degraded | failed
}

Action = Start | Stop | Restart | Enable | Disable | TriggerNow
```

Mapeia 1:1 o mundo real: `my-worker.service` → `kind: service`; `report-daily.timer` →
`kind: scheduled` com `schedule.next_run`. As ações são exatamente as do `systemctl`,
viradas em botões.

## Fluxo de dados

A UI **nunca** faz polling burro nem dispara comando solto — reage a eventos empurrados
pelo núcleo. Mantém o app leve.

1. **Boot** → Registry chama `list()` em cada provider disponível → lista unificada → Overview.
2. **StateWatcher** assina D-Bus/Docker → job muda de estado → evento → UI atualiza + Notifier dispara.
3. **MetricsSampler** (~1–2s, só jobs ativos) → emite `metrics` → sparklines de CPU/mem ao vivo.
4. **Controle** → comando IPC tipado → `provider.control()` → D-Bus (systemd) / socket Docker → estado refrescado. Zero shell.
5. **Logs** → abre job → LogStreamer assina → linhas em tempo real, pesquisáveis.

## UI / UX

Dark-first, near-black, profundidade sutil, tipografia editorial, cor de acento contida,
movimento calmo. Cores de status dessaturadas e sofisticadas: verde (ok), âmbar
(degraded), vermelho (failed).

- **Overview** (tela principal) — lista/grid densa e elegante, agrupada por provider,
  ordenada por saúde (falhas no topo). Cada linha: nome, ícone do kind, pill de estado,
  sparkline ao vivo de CPU/mem, próximo/último run, ações rápidas no hover.
- **Job detail** (slide-over) — status completo, schedule, gráficos de CPU/mem no tempo,
  comando executado, controles, aba de logs ao vivo.
- **Logs** — stream unificado, filtro por job, busca, toggle de follow.
- **Tray** — resumo de saúde ("8 ok · 1 falhou"), toggles rápidos dos jobs-chave, clique abre a janela.
- Estados de loading/erro/vazio desenhados, não improvisados.

## Segurança (desde o commit 1)

- `systemd --user` via **session D-Bus** — sem root.
- **Sem interpolação de shell.** Chamadas D-Bus tipadas (`zbus`) e Docker via `bollard`
  no socket. Onde um binário for inevitável, args fixos, sem string de shell.
- Ações privilegiadas / system units ficam para v2, atrás de **polkit**, explícitas e só
  quando o usuário pedir.
- **Capabilities do Tauri travadas:** o frontend só chama os comandos IPC definidos.
  Nenhum plugin de fs/shell/http exposto amplamente.
- Logs podem conter segredos → **somente locais**, nunca transmitidos. Não há caminho de
  exfiltração no app.
- **Sem telemetria** por padrão.

## Tratamento de erros

- Provider indisponível (sem Docker, sem systemd) → marcado "indisponível", app segue funcionando.
- Ação falha (permissão, unit inexistente) → erro inline com a mensagem real + dica de correção.
- D-Bus/Docker desconecta → reconexão com backoff exponencial; UI mostra "reconectando".
- Erro de amostragem por job é isolado — um job ruim não mata o sampler.

## Testes (nunca para depois)

- **Rust unit** por módulo; `FakeProvider` para a lógica central (registry, sampler,
  eventbus, watcher) testada em isolamento.
- **Integração** `zbus` contra um unit `--user` descartável criado no CI.
- **Integração** `bollard` contra um container de teste (quando Docker presente).
- **Frontend:** testes de componente + contrato contra um mock de IPC.
- **CI:** matriz Linux agora; macOS/Windows quando os providers respectivos entrarem.

## Escopo

**v1 (Linux — sua realidade):** `SystemdUserProvider` + `DockerProvider` (estruturado).
Overview + Job detail + controle + CPU/mem ao vivo + logs unificados + tray +
notificações de falha. Como esta máquina não tem Docker, o caminho que de fato roda no
v1 é o de systemd; o DockerProvider entra ativo quando houver Docker.

**v2:** `SystemdSystemProvider` (polkit) + `CronProvider`.

**v3:** `LaunchdProvider` (macOS) + `WindowsProvider` (Task Scheduler + Services).

Invariante: **o núcleo universal nunca muda; só entram providers novos.**

## Não-objetivos (YAGNI)

- Sem máquinas remotas / SSH no v1 (cockpit é local).
- Sem edição de arquivos `.service`/`.timer` no v1 (só controle de ciclo de vida).
- Sem telemetria, sem conta, sem nuvem.
- Sem redação automática de segredos em logs no v1 (não há exfiltração; fica para depois).
