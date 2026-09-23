<script lang="ts">
  // Two lines on one rating scale: the player's own rating from the game
  // headers, and their strength from move quality (the shrinkage-corrected
  // average of up to the last 20 games' estimates).
  import type { ProgressGame } from '$lib/api/types';
  import { rolling } from '$lib/offline/progress';

  let { games }: { games: ProgressGame[] } = $props();

  const W = 640;
  const H = 220;
  const PAD = { l: 44, r: 12, t: 12, b: 24 };
  const MIN_GAMES = 5;
  let hover = $state<number | null>(null);

  const series = $derived.by(() => {
    const est: number[] = [];
    return games.map((g, i) => {
      if (g.estimate != null) est.push(g.estimate);
      const r = est.length >= MIN_GAMES ? rolling(est) : null;
      return { i, g, strength: r?.[0] ?? null, margin: r?.[1] ?? null, rating: g.user_elo };
    });
  });
  const values = $derived(series.flatMap((p) => [p.strength, p.rating].filter((v): v is number => v != null)));
  const lo = $derived(Math.floor((Math.min(...values) - 100) / 200) * 200);
  const hi = $derived(Math.ceil((Math.max(...values) + 100) / 200) * 200);
  const ticks = $derived(Array.from({ length: Math.round((hi - lo) / 200) + 1 }, (_, k) => lo + k * 200));
  const n = $derived(Math.max(1, games.length - 1));
  const x = (i: number) => PAD.l + (i / n) * (W - PAD.l - PAD.r);
  const y = (v: number) => PAD.t + (1 - (v - lo) / (hi - lo || 1)) * (H - PAD.t - PAD.b);
  function path(key: 'strength' | 'rating') {
    let d = '';
    let pen = false;
    for (const p of series) {
      const v = p[key];
      if (v == null) {
        pen = false;
        continue;
      }
      d += `${pen ? 'L' : 'M'}${x(p.i).toFixed(1)},${y(v).toFixed(1)}`;
      pen = true;
    }
    return d;
  }
  const hovered = $derived(hover === null ? null : series[hover]);

  function nearest(ev: MouseEvent) {
    const r = (ev.currentTarget as SVGElement).getBoundingClientRect();
    const px = ((ev.clientX - r.left) / r.width) * W;
    hover = Math.max(0, Math.min(series.length - 1, Math.round(((px - PAD.l) / (W - PAD.l - PAD.r)) * n)));
  }
</script>

<div class="trend">
  <div class="legend small">
    <span><i class="l1"></i> Your rating (from the games)</span>
    <span><i class="l2"></i> Strength from move quality</span>
  </div>
  <svg viewBox="0 0 {W} {H}" role="img" aria-label="Rating and strength from move quality over your games" onmousemove={nearest} onmouseleave={() => (hover = null)}>
    {#each ticks as t}
      <line x1={PAD.l} x2={W - PAD.r} y1={y(t)} y2={y(t)} class="grid" />
      <text x={PAD.l - 6} y={y(t) + 4} class="axis" text-anchor="end">{t}</text>
    {/each}
    {#if hover !== null}
      <line x1={x(hover)} x2={x(hover)} y1={PAD.t} y2={H - PAD.b} class="cross" />
    {/if}
    <path d={path('rating')} class="s1" />
    <path d={path('strength')} class="s2" />
    {#if hovered}
      {#if hovered.rating != null}<circle cx={x(hovered.i)} cy={y(hovered.rating)} r="4" class="p1" />{/if}
      {#if hovered.strength != null}<circle cx={x(hovered.i)} cy={y(hovered.strength)} r="4" class="p2" />{/if}
    {/if}
    <text x={PAD.l} y={H - 6} class="axis">oldest</text>
    <text x={W - PAD.r} y={H - 6} class="axis" text-anchor="end">latest</text>
  </svg>
  {#if hovered}
    <div class="tip small" style="left: {(x(hovered.i) / W) * 100}%">
      vs {hovered.g.opponent}{#if hovered.g.date} · {hovered.g.date}{/if}
      {#if hovered.rating != null}<br />Rating {hovered.rating}{/if}
      {#if hovered.strength != null}<br />Move quality ≈{hovered.strength} ± {hovered.margin}{/if}
    </div>
  {/if}
</div>

<style>
  .trend {
    --series-1: #2a78d6;
    --series-2: #eb6834;
    position: relative;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme='light'])) .trend {
      --series-1: #3987e5;
      --series-2: #d95926;
    }
  }
  :global(:root[data-theme='dark']) .trend {
    --series-1: #3987e5;
    --series-2: #d95926;
  }
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
  .grid {
    stroke: var(--border);
    stroke-width: 1;
  }
  .axis {
    fill: var(--muted);
    font-size: 11px;
  }
  .cross {
    stroke: var(--muted);
    stroke-dasharray: 3 3;
  }
  .s1,
  .s2 {
    fill: none;
    stroke-width: 2;
    stroke-linejoin: round;
  }
  .s1 {
    stroke: var(--series-1);
  }
  .s2 {
    stroke: var(--series-2);
  }
  .p1,
  .p2 {
    stroke: var(--surface);
    stroke-width: 2;
  }
  .p1 {
    fill: var(--series-1);
  }
  .p2 {
    fill: var(--series-2);
  }
  .legend {
    display: flex;
    gap: 1rem;
    color: var(--muted);
    margin-bottom: 0.25rem;
  }
  .legend i {
    display: inline-block;
    vertical-align: middle;
    margin-right: 0.3rem;
  }
  .legend .l1,
  .legend .l2 {
    width: 14px;
    height: 2px;
  }
  .legend .l1 {
    background: var(--series-1);
  }
  .legend .l2 {
    background: var(--series-2);
  }
  .tip {
    position: absolute;
    top: 1.5rem;
    transform: translateX(-50%);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.3rem 0.5rem;
    box-shadow: var(--shadow);
    pointer-events: none;
    white-space: nowrap;
  }
</style>
