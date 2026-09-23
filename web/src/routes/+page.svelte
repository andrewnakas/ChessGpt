<script lang="ts">
  import { goto } from '$app/navigation';
  import { api } from '$lib/api/client';
  import type { GameSummary } from '$lib/api/types';
  import { onMount } from 'svelte';

  let games = $state<GameSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let filter = $state('');

  onMount(async () => {
    try {
      games = await api.games();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  });

  const shown = $derived(
    games.filter((g) => {
      const f = filter.trim().toLowerCase();
      return !f || [g.white, g.black, g.opening ?? '', g.eco ?? ''].some((s) => s.toLowerCase().includes(f));
    })
  );

  function userAccuracy(g: GameSummary): number | null {
    if (g.user_side === 'white') return g.white_accuracy;
    if (g.user_side === 'black') return g.black_accuracy;
    return null;
  }

  function outcome(g: GameSummary): 'win' | 'loss' | 'draw' | null {
    if (!g.user_side || g.result === '*') return null;
    if (g.result === '1/2-1/2') return 'draw';
    const whiteWon = g.result === '1-0';
    return (g.user_side === 'white') === whiteWon ? 'win' : 'loss';
  }

  async function remove(g: GameSummary, e: Event) {
    e.stopPropagation();
    if (!confirm(`Delete ${g.white} vs ${g.black}?`)) return;
    await api.deleteGame(g.id);
    games = games.filter((x) => x.id !== g.id);
  }
</script>

<div class="head">
  <h1>Your games</h1>
  <input type="text" placeholder="Filter by player or opening" bind:value={filter} />
  <a href="/import"><button class="primary">Import games</button></a>
</div>

{#if loading}
  <p class="muted"><span class="spinner"></span> Loading…</p>
{:else if error}
  <p class="error">{error}</p>
{:else if !games.length}
  <div class="empty card">
    <h2>Start with a game</h2>
    <p>Paste a PGN, or pull your recent games from Lichess or Chess.com. chessgpt runs Stockfish over every move, finds the moments that decided the game, and has the coach explain them at your level.</p>
    <p><a href="/import"><button class="primary">Import games</button></a> or <a href="/board">open the analysis board</a>.</p>
  </div>
{:else}
  <table class="card">
    <thead>
      <tr>
        <th>Players</th>
        <th>Result</th>
        <th class="hide-sm">Opening</th>
        <th class="hide-sm">Date</th>
        <th>Accuracy</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each shown as g (g.id)}
        <tr onclick={() => goto(`/analyse/${g.id}`)} tabindex="0" onkeydown={(e) => e.key === 'Enter' && goto(`/analyse/${g.id}`)}>
          <td>
            <div class:me={g.user_side === 'white'}>♔ {g.white} <span class="muted">{g.white_elo ?? ''}</span></div>
            <div class:me={g.user_side === 'black'}>♚ {g.black} <span class="muted">{g.black_elo ?? ''}</span></div>
          </td>
          <td>
            <span class="res {outcome(g) ?? ''}">{g.result}</span>
          </td>
          <td class="hide-sm">
            {#if g.eco}<span class="mono muted">{g.eco}</span>{/if}
            {g.opening ?? ''}
          </td>
          <td class="hide-sm muted">{g.date ?? ''}</td>
          <td>
            {#if g.analysis_status === 'running' || g.analysis_status === 'queued'}
              <span class="muted"><span class="spinner"></span> analysing</span>
            {:else if g.white_accuracy !== null}
              {#if userAccuracy(g) !== null}
                <b>{userAccuracy(g)?.toFixed(0)}%</b>
              {:else}
                <span class="mono">{g.white_accuracy?.toFixed(0)} / {g.black_accuracy?.toFixed(0)}</span>
              {/if}
            {:else}
              <span class="muted">—</span>
            {/if}
          </td>
          <td><button class="ghost danger" title="Delete" onclick={(e) => remove(g, e)}>✕</button></td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1rem;
  }
  .head h1 {
    margin: 0;
    font-size: 1.5rem;
    white-space: nowrap;
  }
  .head input {
    max-width: 22rem;
  }
  .head a {
    margin-left: auto;
  }
  .empty {
    padding: 1.5rem 2rem;
    max-width: 44rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    overflow: hidden;
  }
  th {
    text-align: left;
    font-size: 0.8rem;
    color: var(--muted);
    font-weight: 600;
    padding: 0.6rem 0.8rem;
    border-bottom: 1px solid var(--border);
  }
  td {
    padding: 0.55rem 0.8rem;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }
  tbody tr {
    cursor: pointer;
  }
  tbody tr:hover {
    background: var(--surface-2);
  }
  .me {
    font-weight: 700;
  }
  .res {
    font-family: var(--mono);
    padding: 0.1rem 0.4rem;
    border-radius: 6px;
  }
  .res.win {
    background: color-mix(in srgb, var(--ok) 20%, transparent);
  }
  .res.loss {
    background: color-mix(in srgb, var(--danger) 20%, transparent);
  }
  .res.draw {
    background: var(--surface-2);
  }
  @media (max-width: 760px) {
    .hide-sm {
      display: none;
    }
    .head {
      flex-wrap: wrap;
    }
  }
</style>
