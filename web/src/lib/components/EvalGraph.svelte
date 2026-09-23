<script lang="ts">
  import type { Classification } from '$lib/api/types';
  import { CLASS_LABEL } from '$lib/chess';

  export interface GraphPoint {
    ply: number;
    /** White win%, 0..100 */
    win: number;
    cls?: Classification;
    key?: boolean;
    san?: string;
  }
  interface Props {
    points: GraphPoint[];
    current: number;
    onselect: (ply: number) => void;
  }
  let { points, current, onselect }: Props = $props();

  const W = 600;
  const H = 110;
  let hover = $state<number | null>(null);

  const maxPly = $derived(Math.max(1, points.length ? points[points.length - 1].ply : 1));
  const x = (ply: number) => (ply / maxPly) * W;
  const y = (win: number) => H - (win / 100) * H;
  const area = $derived.by(() => {
    if (!points.length) return '';
    const line = points.map((p) => `${x(p.ply).toFixed(1)},${y(p.win).toFixed(1)}`).join(' L');
    return `M0,${H} L${line} L${x(points[points.length - 1].ply).toFixed(1)},${H} Z`;
  });

  function plyAt(ev: MouseEvent) {
    const r = (ev.currentTarget as SVGElement).getBoundingClientRect();
    return Math.round(((ev.clientX - r.left) / r.width) * maxPly);
  }
  const hovered = $derived(hover === null ? null : points.find((p) => p.ply === hover) ?? null);
</script>

<div class="graph">
  <svg
    viewBox="0 0 {W} {H}"
    preserveAspectRatio="none"
    role="slider"
    aria-label="Evaluation graph"
    aria-valuemin={0}
    aria-valuemax={maxPly}
    aria-valuenow={current}
    tabindex="-1"
    onmousemove={(e) => (hover = plyAt(e))}
    onmouseleave={() => (hover = null)}
    onclick={(e) => onselect(plyAt(e))}
    onkeydown={(e) => {
      if (e.key === 'ArrowLeft') onselect(Math.max(0, current - 1));
      if (e.key === 'ArrowRight') onselect(Math.min(maxPly, current + 1));
    }}
  >
    <rect width={W} height={H} class="bg" />
    <path d={area} class="area" />
    <line x1="0" x2={W} y1={H / 2} y2={H / 2} class="mid" />
    <line x1={x(current)} x2={x(current)} y1="0" y2={H} class="cursor" />
    {#if hover !== null}
      <line x1={x(hover)} x2={x(hover)} y1="0" y2={H} class="hover" />
    {/if}
    {#each points.filter((p) => p.key || (p.cls && ['blunder', 'mistake', 'missed_win'].includes(p.cls))) as p (p.ply)}
      <circle cx={x(p.ply)} cy={y(p.win)} r={p.key ? 4.5 : 3} class="dot c-{p.cls}" vector-effect="non-scaling-stroke" />
    {/each}
  </svg>
  {#if hovered}
    <div class="tip">
      {hovered.san ?? 'start'}{#if hovered.cls}&nbsp;<span class="c-{hovered.cls}">{CLASS_LABEL[hovered.cls]}</span>{/if}
      · White {hovered.win.toFixed(0)}%
    </div>
  {/if}
</div>

<style>
  .graph {
    position: relative;
  }
  svg {
    display: block;
    width: 100%;
    height: 110px;
    cursor: pointer;
    border-radius: 8px;
    overflow: hidden;
  }
  .bg {
    fill: var(--eval-black);
  }
  .area {
    fill: var(--eval-white);
    opacity: 0.92;
  }
  .mid {
    stroke: var(--muted);
    stroke-dasharray: 4 4;
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
    opacity: 0.6;
  }
  .cursor {
    stroke: var(--accent);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
  .hover {
    stroke: var(--muted);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .dot {
    fill: currentColor;
    stroke: var(--surface);
    stroke-width: 1.5;
  }
  .tip {
    position: absolute;
    top: 4px;
    right: 6px;
    font-size: 0.8rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0 0.4rem;
    pointer-events: none;
  }
</style>
