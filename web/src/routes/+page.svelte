<script lang="ts">
  import { goto } from '$app/navigation';
  import { api } from '$lib/api/client';
  import type { GameSummary } from '$lib/api/types';
  import { offline } from '$lib/offline/backend';
  import { getContext, onMount } from 'svelte';

  const app = getContext<{ guest: boolean; account: unknown }>('app');

  // First visit: pull recent games by username in one step.
  let site = $state<'lichess' | 'chesscom'>('lichess');
  let username = $state('');
  let importing = $state(false);
  async function quickImport(e: Event) {
    e.preventDefault();
    const name = username.trim();
    if (!name) return;
    importing = true;
    error = null;
    try {
      const r = await api.importGames({ source: site, username: name, max: 20 });
      if (r.errors.length && !r.games.length) throw new Error(r.errors[0]);
      for (const g of r.games) {
        const side = g.white.toLowerCase() === name.toLowerCase() ? 'white' : g.black.toLowerCase() === name.toLowerCase() ? 'black' : null;
        if (side && !g.user_side) await api.setSide(g.id, side).catch(() => null);
      }
      if (r.games[0]) await goto(`/analyse/${r.games[0].id}`);
    } catch (err) {
      error = (err as Error).message;
    } finally {
      importing = false;
    }
  }

  // Games saved in this browser (as a guest) before signing in.
  let local = $state(0);
  let moving = $state(false);
  let moved = $state<string | null>(null);
  async function countLocal() {
    if (!app.account) return;
    local = (await offline.games().catch(() => [])).length;
  }
  async function moveLocal() {
    moving = true;
    try {
      const list = await offline.games();
      const pgns = await Promise.all(list.map((g) => offline.game(g.id).then((d) => d.pgn)));
      const r = await api.importGames({ source: 'pgn', pgn: pgns.join('\n\n') });
      for (const g of list) await offline.deleteGame(g.id);
      moved = `Added ${r.games.length} game${r.games.length === 1 ? '' : 's'} to your account.`;
      local = 0;
      games = await api.games();
    } catch (err) {
      error = (err as Error).message;
    } finally {
      moving = false;
    }
  }

  let games = $state<GameSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let filter = $state('');

  onMount(async () => {
    try {
      games = await api.games();
      void countLocal();
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

{#if loading || games.length}
  <div class="head">
    <h1>Your games</h1>
    <input type="text" placeholder="Filter by player or opening" bind:value={filter} />
    <a href="/import"><button class="primary">Import games</button></a>
  </div>
{/if}

{#if local && !moved}
  <div class="card movelocal">
    You have {local} game{local === 1 ? '' : 's'} saved in this browser from before you signed in.
    <button class="primary" onclick={moveLocal} disabled={moving}>{moving ? 'Adding…' : 'Add them to my account'}</button>
  </div>
{/if}
{#if moved}<p class="ok">{moved}</p>{/if}

{#if loading}
  <p class="muted"><span class="spinner"></span> Loading…</p>
{:else if error}
  <p class="error">{error}</p>
{:else if !games.length}
  <section class="hero">
    <h2>Get better at chess from your own games.</h2>
    <p class="lead">
      Stockfish reviews every move, finds the moments that decided the game, and turns your mistakes into training:
      puzzles from your own positions, and drills that teach the same idea several ways until it sticks.
    </p>
    <form class="quick card" onsubmit={quickImport}>
      <select bind:value={site} aria-label="Site">
        <option value="lichess">Lichess</option>
        <option value="chesscom">Chess.com</option>
      </select>
      <input type="text" bind:value={username} placeholder="Your username" autocomplete="username" />
      <button class="primary" type="submit" disabled={importing || !username.trim()}>
        {importing ? 'Fetching games…' : 'Review my games'}
      </button>
    </form>
    <p class="muted small">
      Or <a href="/import">paste a PGN</a>. {#if app.guest}No account needed: it all runs in your browser.{/if}
    </p>
  </section>
  <div class="cards">
    <a class="card tile" href="/drills">
      <b>Drill a tactic</b>
      <span class="muted small">Forks, pins, back-rank mates: spot it, find it, stop it, then convert it against the bot.</span>
    </a>
    <a class="card tile" href="/play">
      <b>Play the bot</b>
      <span class="muted small">A sparring partner from 600 to 2600 that plays like a human at that level.</span>
    </a>
    <a class="card tile" href="/board">
      <b>Analysis board</b>
      <span class="muted small">Set up any position and see Stockfish's best lines.</span>
    </a>
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
  .hero {
    max-width: 46rem;
    margin: 1rem 0 1.5rem;
  }
  .hero h2 {
    font-size: 1.8rem;
    margin: 0 0 0.5rem;
  }
  .lead {
    font-size: 1.05rem;
    line-height: 1.5;
  }
  .quick {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0.7rem;
    margin: 1rem 0 0.4rem;
  }
  .quick input {
    flex: 1 1 12rem;
  }
  .quick select {
    width: auto;
    flex: 0 0 auto;
  }
  .small {
    font-size: 0.85rem;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: 0.8rem;
    max-width: 46rem;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.9rem 1rem;
    color: inherit;
    text-decoration: none;
  }
  .tile:hover {
    border-color: var(--accent);
  }
  .movelocal {
    display: flex;
    flex-wrap: wrap;
    gap: 0.8rem;
    align-items: center;
    padding: 0.7rem 1rem;
    margin-bottom: 1rem;
  }
  .ok {
    color: var(--ok);
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
