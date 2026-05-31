<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import {
    listJobs,
    controlJob,
    jobMetrics,
    type Job,
    type Action,
    type Health,
    type Resources,
  } from "$lib/api";
  import { relativeTime, formatCpu, formatBytes } from "$lib/format";

  let jobs = $state<Job[]>([]);
  let metrics = $state<Record<string, Resources>>({});
  let error = $state<string | null>(null);
  let query = $state("");
  let filter = $state<"all" | "failed" | "active" | "scheduled">("all");
  let busy = $state<Set<string>>(new Set());
  let loaded = $state(false);

  const healthRank: Record<Health, number> = { failed: 0, degraded: 1, ok: 2 };

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
      .sort((a, b) => healthRank[a.health] - healthRank[b.health] || a.name.localeCompare(b.name)),
  );

  async function refresh() {
    try {
      jobs = await listJobs();
      error = null;
      void refreshMetrics();
    } catch (e) {
      error = String(e);
    } finally {
      loaded = true;
    }
  }

  // Métricas (CPU/memória) só fazem sentido para jobs ativos. Buscadas em
  // paralelo; o CPU% precisa de duas amostras, então aparece a partir do 2º ciclo.
  async function refreshMetrics() {
    const active = jobs.filter((j) => j.state === "active");
    const entries = await Promise.all(
      active.map(async (j) => {
        try {
          return [j.id, await jobMetrics(j.id)] as const;
        } catch {
          return [j.id, null] as const;
        }
      }),
    );
    const next: Record<string, Resources> = {};
    for (const [id, r] of entries) if (r) next[id] = r;
    metrics = next;
  }

  async function act(job: Job, action: Action) {
    busy = new Set(busy).add(job.id);
    try {
      await controlJob(job.id, action);
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      const next = new Set(busy);
      next.delete(job.id);
      busy = next;
    }
  }

  let timer: ReturnType<typeof setInterval>;
  onMount(() => {
    refresh();
    timer = setInterval(refresh, 4000);
  });
  onDestroy(() => clearInterval(timer));
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

            <div class="info">
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
                <span class="m"><b>{formatCpu(metrics[job.id].cpuPct)}</b><i>cpu</i></span>
                <span class="m"><b>{formatBytes(metrics[job.id].memBytes)}</b><i>mem</i></span>
              {/if}
            </div>

            <div class="state {job.state}">{job.state}</div>

            <div class="actions">
              {#if job.state === "active"}
                <button onclick={() => act(job, "restart")} title="Reiniciar">↻</button>
                <button class="danger" onclick={() => act(job, "stop")} title="Parar">■</button>
              {:else}
                <button class="go" onclick={() => act(job, "start")} title="Iniciar">▶</button>
              {/if}
              {#if job.kind === "scheduled"}
                <button onclick={() => act(job, "trigger_now")} title="Disparar agora">⚡</button>
              {/if}
              {#if job.enabled}
                <button onclick={() => act(job, "disable")} title="Desabilitar autostart">⦸</button>
              {:else}
                <button onclick={() => act(job, "enable")} title="Habilitar autostart">⦿</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </main>
</div>

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
    gap: 16px;
    width: 132px;
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
