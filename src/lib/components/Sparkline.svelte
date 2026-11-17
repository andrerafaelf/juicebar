<script lang="ts">
  import type { Sample } from "../types";

  let { samples, hours = 24 }: { samples: Sample[]; hours?: number } = $props();

  const W = 300;
  const H = 64;
  const id = `g${Math.random().toString(36).slice(2, 8)}`;

  let view = $derived.by(() => {
    const end = Date.now() / 1000;
    const start = end - hours * 3600;
    const pts = samples.filter((s) => s.t >= start);
    // Carry the last value before the window so the line starts at the left edge.
    const before = samples.filter((s) => s.t < start).at(-1);
    if (before) pts.unshift({ ...before, t: start });
    if (pts.length === 0) return null;
    pts.push({ ...pts[pts.length - 1], t: end });
    const x = (t: number) => ((t - start) / (end - start)) * W;
    const y = (l: number) => H - 4 - (l / 100) * (H - 8);
    // Step line: battery levels change in discrete jumps.
    let d = `M${x(pts[0].t).toFixed(1)},${y(pts[0].level).toFixed(1)}`;
    for (let i = 1; i < pts.length; i++) {
      d += ` H${x(pts[i].t).toFixed(1)} V${y(pts[i].level).toFixed(1)}`;
    }
    const area = `${d} V${H} H${x(pts[0].t).toFixed(1)} Z`;
    const charging = pts
      .slice(0, -1)
      .map((p, i) => ({ p, next: pts[i + 1] }))
      .filter(({ p }) => p.charging)
      .map(({ p, next }) => ({ x: x(p.t), w: Math.max(1, x(next.t) - x(p.t)) }));
    return { d, area, charging };
  });
</script>

{#if view}
  <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" class="spark" role="img" aria-label="Battery level, last {hours} hours">
    <defs>
      <linearGradient id={id} x1="0" x2="0" y1="0" y2="1">
        <stop offset="0" stop-color="var(--accent)" stop-opacity="0.35" />
        <stop offset="1" stop-color="var(--accent)" stop-opacity="0" />
      </linearGradient>
    </defs>
    {#each view.charging as c}
      <rect x={c.x} y="0" width={c.w} height={H} class="charging" />
    {/each}
    <line x1="0" x2={W} y1={H / 2} y2={H / 2} class="grid" />
    <path d={view.area} fill="url(#{id})" />
    <path d={view.d} class="line" vector-effect="non-scaling-stroke" />
  </svg>
{:else}
  <div class="empty">Collecting history…</div>
{/if}

<style>
  .spark {
    width: 100%;
    height: 64px;
    display: block;
  }
  .line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
  }
  .grid {
    stroke: var(--line);
    stroke-dasharray: 2 4;
    vector-effect: non-scaling-stroke;
  }
  .charging {
    fill: var(--charging);
    opacity: 0.12;
  }
  .empty {
    height: 64px;
    display: grid;
    place-items: center;
    color: var(--text-3);
    font-size: 12px;
  }
</style>
