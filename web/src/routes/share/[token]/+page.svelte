<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { getContext, onMount } from 'svelte';
  import { api, type Account } from '$lib/api/client';
  import type { GameDetail } from '$lib/api/types';
  import Board from '$lib/components/Board.svelte';
  import EvalBar from '$lib/components/EvalBar.svelte';
  import EvalGraph, { type GraphPoint } from '$lib/components/EvalGraph.svelte';
  import MoveList from '$lib/components/MoveList.svelte';
  import { CLASS_LABEL, fmtScore, uciSquares, whiteWin } from '$lib/chess';

  const app = getContext<{ account: Account | null; accounts: boolean }>('app');
  const token = $derived(page.params.token as string);
  let game = $state<GameDetail | null>(null);
  let error = $state<string | null>(null);
  let ply = $state(0);
  let saving = $state(false);

  onMount(async () => {
    try {
      game = await api.shared(token);
    } catch (e) {
      error = (e as Error).message;
    }
  });

  const evals = $derived(new Map((game?.analysis?.moves ?? []).map((m) => [m.ply, m])));
  const fen = $derived(!game ? '' : ply === 0 ? game.start_fen : game.moves[ply - 1].fen_after);
  const lastMove = $derived(game && ply > 0 ? uciSquares(game.moves[ply - 1].uci) : null);
  const cur = $derived(evals.get(ply));
  const graph: GraphPoint[] = $derived(
    (game?.analysis?.moves ?? []).map((m) => ({
      ply: m.ply,
      win: m.mover === 'white' ? m.win_after : 100 - m.win_after,
      cls: m.classification,
      key: m.is_key_moment,
      san: m.san
    }))
  );

  async function save() {
    if (app.accounts && !app.account) {
      goto(`/login?return=${encodeURIComponent(page.url.pathname)}`);
      return;
    }
    saving = true;
    try {
      const r = await api.claim(token);
      goto(`/analyse/${r.game_id}`);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      saving = false;
    }
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (!game) return;
    if (e.key === 'ArrowLeft') ply = Math.max(0, ply - 1);
    if (e.key === 'ArrowRight') ply = Math.min(game.moves.length, ply + 1);
  }}
/>

{#if error}
  <p class="error">{error}</p>
{:else if !game}
  <p class="muted"><span class="spinner"></span> Loading…</p>
{:else}
  <div class="layout">
    <div class="boardrow">
      <EvalBar win={cur ? whiteWin(cur.score) : 50} label={cur ? fmtScore(cur.score) : ''} />
      <Board {fen} {lastMove} interactive={false} />
    </div>
    <div class="side">
      <div class="card pad">
        <div class="title"><b>{game.summary.white}</b> vs <b>{game.summary.black}</b> <span class="mono">{game.summary.result}</span></div>
        <div class="muted small">{game.summary.opening ?? ''}</div>
        {#if game.analysis?.white_accuracy != null}
          <div class="small">Accuracy: White {game.analysis.white_accuracy.toFixed(1)}% · Black {game.analysis.black_accuracy?.toFixed(1)}%</div>
        {/if}
        {#if cur}<div class="small">{cur.san}: <span class="c-{cur.classification}">{CLASS_LABEL[cur.classification]}</span>{#if cur.best_san && cur.best_san !== cur.san}, engine preferred {cur.best_san}{/if}</div>{/if}
        <button class="primary" onclick={save} disabled={saving}>Save to my chessgpt and open the coach</button>
      </div>
      <div class="card pad moves"><MoveList moves={game.moves} {evals} current={ply} onselect={(p) => (ply = p)} /></div>
      {#if graph.length}<EvalGraph points={graph} current={ply} onselect={(p) => (ply = p)} />{/if}
    </div>
  </div>
{/if}

<style>
  .layout {
    display: grid;
    grid-template-columns: minmax(300px, min(calc(100vh - 160px), 640px)) minmax(300px, 1fr);
    gap: 1.2rem;
    align-items: start;
  }
  .boardrow {
    display: grid;
    grid-template-columns: 22px 1fr;
    gap: 6px;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .pad {
    padding: 0.8rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .small {
    font-size: 0.88rem;
  }
  .moves {
    max-height: 18rem;
    overflow: hidden;
    display: flex;
  }
  .moves :global(.moves) {
    flex: 1;
  }
  @media (max-width: 900px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }
</style>
