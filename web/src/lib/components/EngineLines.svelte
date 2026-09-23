<script lang="ts">
  import type { EngineAnalysis } from '$lib/api/types';
  import { fmtScore, numberedLine } from '$lib/chess';

  interface Props {
    analysis: EngineAnalysis | null;
    on: boolean;
    error?: string | null;
    ontoggle: () => void;
    onplay: (uci: string[]) => void;
  }
  let { analysis, on, error = null, ontoggle, onplay }: Props = $props();
</script>

<div class="engine">
  <div class="head">
    <label class="toggle">
      <input type="checkbox" checked={on} onchange={ontoggle} />
      <span>Stockfish</span>
    </label>
    {#if on && analysis}
      <span class="muted small">
        {#if analysis.terminal}{analysis.terminal}{:else}depth {analysis.depth}{#if !analysis.done}&nbsp;<span class="spinner"></span>{/if}
          · {(analysis.nps / 1000).toFixed(0)} kn/s{/if}
      </span>
    {:else if on}
      <span class="muted small"><span class="spinner"></span> starting…</span>
    {:else}
      <span class="muted small">off (space)</span>
    {/if}
  </div>
  {#if error}<div class="error small">{error}</div>{/if}
  {#if on && analysis}
    {#each analysis.lines as l (l.rank)}
      <button class="line" onclick={() => onplay(l.pv_uci)} title="Play this line on the board">
        <span class="score mono">{fmtScore(l.score)}</span>
        <span class="pv mono">{numberedLine(analysis.fen, l.pv_san.slice(0, 12))}</span>
      </button>
    {/each}
  {/if}
</div>

<style>
  .engine {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-weight: 600;
    cursor: pointer;
  }
  .small {
    font-size: 0.8rem;
  }
  .line {
    display: flex;
    gap: 0.6rem;
    text-align: left;
    background: none;
    border: none;
    padding: 0.15rem 0.3rem;
    border-radius: 6px;
    overflow: hidden;
  }
  .line:hover {
    background: var(--surface-2);
  }
  .score {
    min-width: 3.3rem;
    font-weight: 700;
  }
  .pv {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 0.88rem;
    color: var(--muted);
  }
</style>
