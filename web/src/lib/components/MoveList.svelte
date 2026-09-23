<script lang="ts">
  import type { MoveEval, PlyMove } from '$lib/api/types';
  import { CLASS_GLYPH, CLASS_LABEL } from '$lib/chess';

  interface Props {
    moves: PlyMove[];
    evals: Map<number, MoveEval>;
    current: number;
    onselect: (ply: number) => void;
  }
  let { moves, evals, current, onselect }: Props = $props();

  interface Row {
    n: number;
    white?: PlyMove;
    black?: PlyMove;
  }
  const rows = $derived.by(() => {
    const out: Row[] = [];
    for (const m of moves) {
      const fullmove = parseInt(m.fen_before.split(' ')[5] ?? '1', 10);
      if (m.mover === 'white') out.push({ n: fullmove, white: m });
      else {
        const last = out[out.length - 1];
        if (last && !last.black && last.n === fullmove) last.black = m;
        else out.push({ n: fullmove, black: m });
      }
    }
    return out;
  });

  let list: HTMLDivElement;
  $effect(() => {
    const el = list?.querySelector<HTMLElement>(`[data-ply="${current}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  });
</script>

{#snippet cell(m: PlyMove | undefined)}
  {#if m}
    {@const e = evals.get(m.ply)}
    <button
      class="mv"
      class:active={current === m.ply}
      class:key={e?.is_key_moment}
      data-ply={m.ply}
      title={e ? `${CLASS_LABEL[e.classification]}` : ''}
      onclick={() => onselect(m.ply)}
    >
      <span class="san c-{e?.classification}">{m.san}{e ? CLASS_GLYPH[e.classification] : ''}</span>
      {#if e?.is_key_moment}<span class="star" aria-label="key moment">●</span>{/if}
    </button>
  {:else}
    <span class="mv empty">…</span>
  {/if}
{/snippet}

<div class="moves" bind:this={list}>
  <button class="mv start" class:active={current === 0} data-ply="0" onclick={() => onselect(0)}>Start</button>
  {#each rows as r (r.n + (r.white ? 'w' : 'b'))}
    <div class="row">
      <span class="n">{r.n}.</span>
      {@render cell(r.white)}
      {@render cell(r.black)}
    </div>
  {/each}
</div>

<style>
  .moves {
    overflow-y: auto;
    max-height: 100%;
    font-family: var(--mono);
    font-size: 0.92rem;
  }
  .row {
    display: grid;
    grid-template-columns: 2.6rem 1fr 1fr;
    align-items: center;
  }
  .n {
    color: var(--muted);
    text-align: right;
    padding-right: 0.5rem;
  }
  .mv {
    border: none;
    background: none;
    text-align: left;
    padding: 0.18rem 0.45rem;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 1.8rem;
  }
  .mv:hover {
    background: var(--surface-2);
  }
  .mv.active {
    background: var(--accent);
    color: var(--accent-contrast);
  }
  .mv.active .san {
    color: inherit;
  }
  .mv.start {
    font-family: var(--font);
    font-size: 0.8rem;
    color: var(--muted);
    margin-bottom: 0.2rem;
  }
  .mv.start.active {
    color: var(--accent-contrast);
  }
  .san.c-good,
  .san.c-book,
  .san.c-best,
  .san.c-excellent {
    color: inherit;
  }
  .san.c-best::after,
  .san.c-excellent::after {
    content: '';
  }
  .star {
    font-size: 0.55rem;
    color: var(--accent);
  }
  .active .star {
    color: inherit;
  }
  .empty {
    color: var(--muted);
  }
</style>
