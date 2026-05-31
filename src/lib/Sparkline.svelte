<script lang="ts">
  // Mini-gráfico de linha. Auto-escala para o máximo da janela (mostra a forma).
  let {
    values,
    color = "var(--accent)",
    width = 56,
    height = 18,
  }: { values: number[]; color?: string; width?: number; height?: number } = $props();

  function points(vs: number[]): string {
    if (vs.length < 2) return "";
    const hi = Math.max(...vs, 0.0001);
    const step = width / (vs.length - 1);
    return vs
      .map((v, i) => `${(i * step).toFixed(1)},${(height - (v / hi) * (height - 2) - 1).toFixed(1)}`)
      .join(" ");
  }

  const pts = $derived(points(values));
</script>

{#if values.length >= 2}
  <svg {width} {height} viewBox="0 0 {width} {height}" class="spark" aria-hidden="true">
    <polyline
      points={pts}
      fill="none"
      stroke={color}
      stroke-width="1.5"
      stroke-linejoin="round"
      stroke-linecap="round"
    />
  </svg>
{:else}
  <span class="spark-empty" style:width="{width}px"></span>
{/if}

<style>
  .spark {
    display: block;
  }
  .spark-empty {
    display: inline-block;
    height: 1px;
  }
</style>
