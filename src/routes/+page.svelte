<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    listJobs,
    controlJob,
    jobDetail,
    jobLogs,
    type Job,
    type Action,
    type Health,
    type Resources,
    type JobDetail,
  } from "$lib/api";
  import { relativeTime, formatCpu, formatBytes } from "$lib/format";
  import Sparkline from "$lib/Sparkline.svelte";

  const HISTORY = 40; // pontos mantidos por job

  let jobs = $state<Job[]>([]);
  let metrics = $state<Record<string, Resources>>({});
  let history = $state<Record<string, { cpu: number[]; mem: number[] }>>({});
  let error = $state<string | null>(null);
  let query = $state("");
  let filter = $state<"all" | "failed" | "active" | "scheduled">("all");
  let sort = $state<"health" | "activity">("health");
  let busy = $state<Set<string>>(new Set());
  let loaded = $state(false);
  let confirm = $state<{ job: Job; action: Action } | null>(null);

  // Painel de detalhe
  let selectedId = $state<string | null>(null);
  let detail = $state<JobDetail | null>(null);
  let logs = $state<string[]>([]);
  let followLogs = $state(true);
  let logsTimer: ReturnType<typeof setInterval> | null = null;
  // Job sempre fresco (reflete os eventos `jobs`), derivado do id selecionado.
  const selected = $derived(selectedId ? (jobs.find((j) => j.id === selectedId) ?? null) : null);
  const failInfo = $derived(
    detail && detail.exitReason && detail.exitReason !== "success"
      ? `${detail.exitReason}${detail.exitCode != null ? ` (código ${detail.exitCode})` : ""}`
      : null,
  );
  let logsEl = $state<HTMLPreElement | null>(null);
  $effect(() => {
    logs; // dependência
    if (logsEl && followLogs) logsEl.scrollTop = logsEl.scrollHeight;
  });

  const healthRank: Record<Health, number> = { failed: 0, degraded: 1, ok: 2 };
  // Ordenação por atividade: ativos no topo, inativos embaixo.
  const activityRank: Record<string, number> = {
    active: 0,
    activating: 1,
    deactivating: 1,
    unknown: 2,
    inactive: 3,
    failed: 4,
  };

  // Rótulos e quais ações exigem confirmação (as que EXECUTAM algo).
  // Parar é imediato de propósito — para você conseguir abortar rápido.
  const ACTION_LABEL: Record<Action, string> = {
    start: "Iniciar",
    stop: "Parar",
    restart: "Reiniciar",
    enable: "Habilitar autostart",
    disable: "Desabilitar autostart",
    trigger_now: "Disparar agora",
  };
  const CONFIRM_ACTIONS: Action[] = ["start", "restart", "trigger_now"];

  const running = (s: Job["state"]) =>
    s === "active" || s === "activating" || s === "deactivating";

  const counts = $derived({
    total: jobs.length,
    failed: jobs.filter((j) => j.health === "failed").length,
    active: jobs.filter((j) => j.state === "active").length,
  });

  const visible = $derived(
    jobs
      .filter((j) => {
        if (filter === "failed" && j.health !== "failed") return false;
        if (filter === "active" && j.state !== "active") return false;
        if (filter === "scheduled" && j.kind !== "scheduled") return false;
        const q = query.trim().toLowerCase();
        if (q && !j.name.toLowerCase().includes(q) && !j.description.toLowerCase().includes(q))
          return false;
        return true;
      })
      .sort((a, b) => {
        const primary =
          sort === "activity"
            ? activityRank[a.state] - activityRank[b.state]
            : healthRank[a.health] - healthRank[b.health];
        return primary || a.name.localeCompare(b.name);
      }),
  );

  // Carga inicial; depois o estado chega por eventos do backend (sem polling).
  async function reload() {
    try {
      jobs = await listJobs();
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loaded = true;
    }
  }

  // Ponto de entrada dos botões: pede confirmação nas ações que executam algo.
  function request(job: Job, action: Action) {
    if (CONFIRM_ACTIONS.includes(action)) {
      confirm = { job, action };
    } else {
      act(job, action);
    }
  }

  function confirmYes() {
    if (confirm) {
      const { job, action } = confirm;
      confirm = null;
      act(job, action);
    }
  }

  async function act(job: Job, action: Action) {
    busy = new Set(busy).add(job.id);
    try {
      await controlJob(job.id, action);
      await reload(); // feedback imediato; o evento `jobs` também confirmará
    } catch (e) {
      error = String(e);
    } finally {
      const next = new Set(busy);
      next.delete(job.id);
      busy = next;
    }
  }

  // --- Detalhe + logs ---
  async function openDetail(job: Job) {
    selectedId = job.id;
    detail = null;
    logs = [];
    try {
      detail = await jobDetail(job.id);
    } catch (e) {
      error = String(e);
    }
    await fetchLogs();
    startFollow();
  }

  async function fetchLogs() {
    if (!selectedId) return;
    try {
      logs = await jobLogs(selectedId, 300);
    } catch (e) {
      logs = [`(sem logs: ${e})`];
    }
  }

  function startFollow() {
    stopFollow();
    if (followLogs) logsTimer = setInterval(fetchLogs, 2000);
  }
  function stopFollow() {
    if (logsTimer) {
      clearInterval(logsTimer);
      logsTimer = null;
    }
  }
  function toggleFollow() {
    followLogs = !followLogs;
    startFollow();
  }
  function closeDetail() {
    selectedId = null;
    detail = null;
    logs = [];
    stopFollow();
  }

  // Push do backend: `jobs` (mudança de estado) e `metrics` (CPU/mem a cada 2s).
  let unlisten: UnlistenFn[] = [];
  onMount(async () => {
    await reload();
    unlisten.push(
      await listen<Job[]>("jobs", (e) => {
        jobs = e.payload;
        loaded = true;
      }),
    );
    unlisten.push(
      await listen<Record<string, Resources>>("metrics", (e) => {
        metrics = e.payload;
        const h = { ...history };
        for (const [id, r] of Object.entries(e.payload)) {
          const prev = h[id] ?? { cpu: [], mem: [] };
          h[id] = {
            cpu: [...prev.cpu, r.cpuPct ?? 0].slice(-HISTORY),
            mem: [...prev.mem, r.memBytes ?? 0].slice(-HISTORY),
          };
        }
        history = h;
      }),
    );
  });
  onDestroy(() => {
    unlisten.forEach((u) => u());
    stopFollow();
  });

  function onEscape() {
    if (confirm) confirm = null;
    else if (selectedId) closeDetail();
  }
</script>

<div class="app">
  <header>
    <div class="brand">
      <span class="mark">◇</span>
      <h1>Maestro</h1>
      <span class="tagline">cockpit de jobs de background</span>
    </div>
    <div class="summary">
      <span class="stat"><b>{counts.total}</b> jobs</span>
      <span class="stat ok"><b>{counts.active}</b> ativos</span>
      <span class="stat fail" class:dim={counts.failed === 0}><b>{counts.failed}</b> falhando</span>
    </div>
  </header>

  <div class="toolbar">
    <input class="search" placeholder="Buscar serviço…" bind:value={query} />
    <div class="chips">
      {#each [["all", "Todos"], ["failed", "Falhas"], ["active", "Ativos"], ["scheduled", "Agendados"]] as [key, label]}
        <button class="chip" class:on={filter === key} onclick={() => (filter = key as typeof filter)}>
          {label}
        </button>
      {/each}
    </div>
  </div>

  <div class="sortbar">
    <span class="sortlabel">Ordenar:</span>
    <div class="seg">
      <button class:on={sort === "health"} onclick={() => (sort = "health")}>Saúde</button>
      <button class:on={sort === "activity"} onclick={() => (sort = "activity")}>Atividade</button>
    </div>
  </div>

  {#if error}
    <div class="error">{error}</div>
  {/if}

  <main>
    {#if !loaded}
      <div class="empty">Carregando…</div>
    {:else if visible.length === 0}
      <div class="empty">Nenhum job para este filtro.</div>
    {:else}
      <ul class="list">
        {#each visible as job (job.id)}
          <li class="row" class:busy={busy.has(job.id)}>
            <span class="dot {job.health}" title={job.state}></span>

            <div
              class="info"
              role="button"
              tabindex="0"
              onclick={() => openDetail(job)}
              onkeydown={(e) => (e.key === "Enter" || e.key === " ") && openDetail(job)}
            >
              <div class="line1">
                <span class="name">{job.name}</span>
                <span class="kind">{job.kind}</span>
                {#if job.enabled}<span class="badge">autostart</span>{/if}
              </div>
              <div class="line2">
                <span class="desc">{job.description || "—"}</span>
                {#if job.kind === "scheduled" && job.schedule}
                  <span class="sched">
                    próx {relativeTime(job.schedule.nextRun)} · últ {relativeTime(job.schedule.lastRun)}
                  </span>
                {/if}
              </div>
            </div>

            <div class="metrics">
              {#if metrics[job.id]}
                {#if (history[job.id]?.cpu.length ?? 0) > 1}
                  <Sparkline values={history[job.id].cpu} color="var(--accent)" width={44} height={22} />
                {/if}
                <span class="m"><b>{formatCpu(metrics[job.id].cpuPct)}</b><i>cpu</i></span>
                <span class="m"><b>{formatBytes(metrics[job.id].memBytes)}</b><i>mem</i></span>
              {/if}
            </div>

            <div class="state {job.state}">{job.state}</div>

            <div class="actions">
              {#if running(job.state)}
                <button onclick={() => request(job, "restart")} title="Reiniciar">↻</button>
                <button class="danger" onclick={() => request(job, "stop")} title="Parar">■</button>
              {:else}
                <button class="go" onclick={() => request(job, "start")} title="Iniciar">▶</button>
              {/if}
              {#if job.kind === "scheduled"}
                <button onclick={() => request(job, "trigger_now")} title="Disparar agora">⚡</button>
              {/if}
              {#if job.enabled}
                <button onclick={() => request(job, "disable")} title="Desabilitar autostart">⦸</button>
              {:else}
                <button onclick={() => request(job, "enable")} title="Habilitar autostart">⦿</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </main>
</div>

<svelte:window onkeydown={(e) => e.key === "Escape" && onEscape()} />

{#if selected}
  <div class="drawer-backdrop" onclick={closeDetail} role="presentation"></div>
  <aside class="drawer">
    <header class="dh">
      <div class="dh-main">
        <div class="dtitle">
          <span class="dot {selected.health}"></span>
          <h2>{selected.name}</h2>
        </div>
        <span class="kind">{selected.kind}</span>
        {#if selected.enabled}<span class="badge">autostart</span>{/if}
      </div>
      <button class="close" onclick={closeDetail} aria-label="Fechar">✕</button>
    </header>

    <div class="dbody">
      <p class="ddesc">{selected.description || "—"}</p>

      <dl class="meta">
        {#if failInfo}
          <dt>Falha</dt>
          <dd class="fail">{failInfo}</dd>
        {/if}
        {#if detail?.command}
          <dt>Comando</dt>
          <dd class="mono wrap">{detail.command}</dd>
        {/if}
        {#if detail?.fragmentPath}
          <dt>Arquivo</dt>
          <dd class="mono wrap">{detail.fragmentPath}</dd>
        {/if}
        {#if selected.kind === "scheduled" && selected.schedule}
          <dt>Próximo</dt>
          <dd>{relativeTime(selected.schedule.nextRun)}</dd>
          <dt>Último</dt>
          <dd>{relativeTime(selected.schedule.lastRun)}</dd>
        {/if}
        {#if detail?.since}
          <dt>Ativo desde</dt>
          <dd>{relativeTime(detail.since)}</dd>
        {/if}
        {#if metrics[selected.id]}
          <dt>CPU</dt>
          <dd>{formatCpu(metrics[selected.id].cpuPct)}</dd>
          <dt>Memória</dt>
          <dd>{formatBytes(metrics[selected.id].memBytes)}</dd>
        {/if}
      </dl>

      {#if selectedId && (history[selectedId]?.cpu.length ?? 0) > 1}
        <div class="graphs">
          <div class="graph">
            <div class="glabel"><span>CPU</span><b>{formatCpu(metrics[selectedId]?.cpuPct)}</b></div>
            <Sparkline values={history[selectedId].cpu} color="var(--accent)" width={416} height={46} />
          </div>
          <div class="graph">
            <div class="glabel"><span>Memória</span><b>{formatBytes(metrics[selectedId]?.memBytes ?? null)}</b></div>
            <Sparkline values={history[selectedId].mem} color="var(--ok)" width={416} height={46} />
          </div>
        </div>
      {/if}

      <div class="logs-head">
        <h3>Logs</h3>
        <button class="follow" class:on={followLogs} onclick={toggleFollow}>
          {followLogs ? "● ao vivo" : "pausado"}
        </button>
      </div>
      <pre class="logs" bind:this={logsEl}>{logs.length ? logs.join("\n") : "(sem logs)"}</pre>
    </div>
  </aside>
{/if}

{#if confirm}
  <div class="overlay" onclick={() => (confirm = null)} role="presentation">
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      tabindex="-1"
    >
      <h2>{ACTION_LABEL[confirm.action]}?</h2>
      <p>
        Confirmar <b>{ACTION_LABEL[confirm.action].toLowerCase()}</b> de
        <span class="mono">{confirm.job.name}</span>?
        {#if confirm.action !== "stop"}
          <span class="warn">Isso vai executar o serviço agora.</span>
        {/if}
      </p>
      <div class="modal-actions">
        <button class="btn ghost" onclick={() => (confirm = null)}>Cancelar</button>
        <button class="btn primary" onclick={confirmYes}>{ACTION_LABEL[confirm.action]}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  :global(:root) {
    --bg: #0a0b0d;
    --panel: #101216;
    --panel-2: #15181e;
    --line: rgba(255, 255, 255, 0.07);
    --line-strong: rgba(255, 255, 255, 0.12);
    --text: #e7e9ee;
    --text-dim: #8b909b;
    --text-faint: #5a5f6a;
    --accent: #7c6cf0;
    --ok: #58c98a;
    --degraded: #e6b450;
    --fail: #ef6f6f;
    font-family: Inter, -apple-system, "Segoe UI", Roboto, sans-serif;
    -webkit-font-smoothing: antialiased;
  }
  :global(body) {
    margin: 0;
    background: var(--bg);
    color: var(--text);
  }
  :global(*) {
    box-sizing: border-box;
  }

  .app {
    max-width: 980px;
    margin: 0 auto;
    padding: 28px 24px 64px;
  }

  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    padding-bottom: 18px;
    border-bottom: 1px solid var(--line);
  }
  .brand {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .mark {
    color: var(--accent);
    font-size: 18px;
  }
  h1 {
    font-size: 19px;
    font-weight: 620;
    letter-spacing: -0.02em;
    margin: 0;
  }
  .tagline {
    color: var(--text-faint);
    font-size: 12px;
  }
  .summary {
    display: flex;
    gap: 16px;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .stat b {
    color: var(--text);
    font-weight: 600;
  }
  .stat.ok b {
    color: var(--ok);
  }
  .stat.fail b {
    color: var(--fail);
  }
  .stat.dim {
    opacity: 0.4;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 14px;
    margin: 18px 0;
  }
  .search {
    flex: 1;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 9px;
    color: var(--text);
    padding: 9px 13px;
    font-size: 13px;
    outline: none;
    transition: border-color 0.15s;
  }
  .search:focus {
    border-color: var(--accent);
  }
  .chips {
    display: flex;
    gap: 6px;
  }
  .chip {
    background: transparent;
    border: 1px solid var(--line);
    color: var(--text-dim);
    border-radius: 7px;
    padding: 7px 12px;
    font-size: 12.5px;
    cursor: pointer;
    transition: all 0.15s;
  }
  .chip:hover {
    color: var(--text);
    border-color: var(--line-strong);
  }
  .chip.on {
    background: var(--panel-2);
    color: var(--text);
    border-color: var(--line-strong);
  }

  .sortbar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: -6px 0 16px;
  }
  .sortlabel {
    font-size: 12px;
    color: var(--text-faint);
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--line);
    border-radius: 7px;
    overflow: hidden;
  }
  .seg button {
    background: transparent;
    border: none;
    color: var(--text-dim);
    padding: 6px 12px;
    font-size: 12.5px;
    cursor: pointer;
    transition: all 0.15s;
  }
  .seg button:hover {
    color: var(--text);
  }
  .seg button.on {
    background: var(--panel-2);
    color: var(--text);
  }
  .seg button:first-child {
    border-right: 1px solid var(--line);
  }

  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(2px);
    display: grid;
    place-items: center;
    z-index: 50;
    animation: fade 0.12s ease;
  }
  .modal {
    width: 420px;
    max-width: calc(100vw - 40px);
    background: var(--panel);
    border: 1px solid var(--line-strong);
    border-radius: 14px;
    padding: 22px 22px 18px;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.5);
    animation: pop 0.14s ease;
  }
  .modal h2 {
    margin: 0 0 8px;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .modal p {
    margin: 0 0 18px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .mono {
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 12px;
    color: var(--text);
    background: var(--panel-2);
    padding: 1px 6px;
    border-radius: 5px;
  }
  .warn {
    display: block;
    margin-top: 8px;
    color: var(--degraded);
    font-size: 12px;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    border-radius: 8px;
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 540;
    cursor: pointer;
    border: 1px solid var(--line);
    transition: all 0.12s;
  }
  .btn.ghost {
    background: transparent;
    color: var(--text-dim);
  }
  .btn.ghost:hover {
    color: var(--text);
    border-color: var(--line-strong);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }
  .btn.primary:hover {
    filter: brightness(1.1);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.96);
    }
  }

  /* --- Drawer de detalhe --- */
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 40;
    animation: fade 0.12s ease;
  }
  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: 480px;
    max-width: 100vw;
    background: var(--panel);
    border-left: 1px solid var(--line-strong);
    z-index: 45;
    display: flex;
    flex-direction: column;
    box-shadow: -24px 0 60px rgba(0, 0, 0, 0.4);
    animation: slide 0.16s ease;
  }
  @keyframes slide {
    from {
      transform: translateX(20px);
      opacity: 0;
    }
  }
  .dh {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    padding: 18px 20px;
    border-bottom: 1px solid var(--line);
  }
  .dh-main {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .dtitle {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .dtitle h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    word-break: break-all;
  }
  .close {
    background: transparent;
    border: none;
    color: var(--text-faint);
    font-size: 15px;
    cursor: pointer;
    padding: 4px;
    line-height: 1;
  }
  .close:hover {
    color: var(--text);
  }
  .dbody {
    flex: 1;
    overflow-y: auto;
    padding: 18px 20px 24px;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .ddesc {
    margin: 0 0 16px;
    font-size: 13px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .meta {
    display: grid;
    grid-template-columns: 92px 1fr;
    gap: 8px 14px;
    margin: 0 0 20px;
    font-size: 12.5px;
  }
  .meta dt {
    color: var(--text-faint);
  }
  .meta dd {
    margin: 0;
    color: var(--text);
  }
  .meta dd.wrap {
    word-break: break-all;
  }
  .meta dd.fail {
    color: var(--fail);
  }

  .graphs {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-bottom: 20px;
  }
  .graph {
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 9px;
    padding: 10px 12px;
  }
  .glabel {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 4px;
  }
  .glabel span {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }
  .glabel b {
    font-size: 13px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .logs-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .logs-head h3 {
    margin: 0;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }
  .follow {
    background: transparent;
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--text-faint);
    font-size: 11px;
    padding: 4px 9px;
    cursor: pointer;
  }
  .follow.on {
    color: var(--ok);
    border-color: rgba(88, 201, 138, 0.3);
  }
  .logs {
    flex: 1;
    min-height: 180px;
    margin: 0;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 12px;
    overflow: auto;
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 11px;
    line-height: 1.55;
    color: var(--text-dim);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .error {
    background: rgba(239, 111, 111, 0.1);
    border: 1px solid rgba(239, 111, 111, 0.3);
    color: var(--fail);
    padding: 10px 14px;
    border-radius: 9px;
    font-size: 12.5px;
    margin-bottom: 14px;
  }

  .empty {
    text-align: center;
    color: var(--text-faint);
    padding: 60px 0;
    font-size: 13px;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: 12px;
    overflow: hidden;
    background: var(--panel);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 13px 16px;
    border-bottom: 1px solid var(--line);
    transition: background 0.12s;
  }
  .row:last-child {
    border-bottom: none;
  }
  .row:hover {
    background: var(--panel-2);
  }
  .row.busy {
    opacity: 0.5;
    pointer-events: none;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .dot.ok {
    background: var(--ok);
    box-shadow: 0 0 8px rgba(88, 201, 138, 0.5);
  }
  .dot.degraded {
    background: var(--degraded);
  }
  .dot.failed {
    background: var(--fail);
    box-shadow: 0 0 8px rgba(239, 111, 111, 0.5);
  }

  .info {
    flex: 1;
    min-width: 0;
    cursor: pointer;
    outline: none;
  }
  .info:focus-visible .name {
    color: var(--accent);
  }
  .line1 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    font-size: 13.5px;
    font-weight: 540;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kind {
    font-size: 10.5px;
    color: var(--text-faint);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    border: 1px solid var(--line);
    border-radius: 5px;
    padding: 1px 5px;
  }
  .badge {
    font-size: 10px;
    color: var(--accent);
    border: 1px solid rgba(124, 108, 240, 0.3);
    border-radius: 5px;
    padding: 1px 5px;
  }
  .line2 {
    display: flex;
    gap: 12px;
    margin-top: 3px;
  }
  .desc {
    font-size: 12px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 420px;
  }
  .sched {
    font-size: 11.5px;
    color: var(--text-faint);
    white-space: nowrap;
  }

  .metrics {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 188px;
    flex-shrink: 0;
    justify-content: flex-end;
  }
  .m {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    line-height: 1.25;
  }
  .m b {
    font-size: 12.5px;
    font-weight: 560;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .m i {
    font-size: 9.5px;
    font-style: normal;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }

  .state {
    font-size: 11.5px;
    color: var(--text-dim);
    text-transform: capitalize;
    width: 86px;
    text-align: right;
  }
  .state.failed {
    color: var(--fail);
  }
  .state.active {
    color: var(--ok);
  }

  .actions {
    display: flex;
    gap: 5px;
    /* Largura fixa (4 botões: 4×30 + 3×5 de gap) reservada sempre, para a coluna
       de estado alinhar entre linhas com nº de ações diferente. */
    width: 135px;
    justify-content: flex-end;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity 0.12s;
  }
  .row:hover .actions {
    opacity: 1;
  }
  .actions button {
    background: var(--panel-2);
    border: 1px solid var(--line);
    color: var(--text-dim);
    width: 30px;
    height: 30px;
    border-radius: 7px;
    cursor: pointer;
    font-size: 13px;
    display: grid;
    place-items: center;
    transition: all 0.12s;
  }
  .actions button:hover {
    color: var(--text);
    border-color: var(--line-strong);
    background: #1c2026;
  }
  .actions button.go:hover {
    color: var(--ok);
    border-color: rgba(88, 201, 138, 0.4);
  }
  .actions button.danger:hover {
    color: var(--fail);
    border-color: rgba(239, 111, 111, 0.4);
  }
</style>
