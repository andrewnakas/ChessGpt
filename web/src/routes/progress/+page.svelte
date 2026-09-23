<script lang="ts">
  import { api } from '$lib/api/client';
  import type { Progress } from '$lib/api/types';
  import RatingTrend from '$lib/components/RatingTrend.svelte';
  import { onMount } from 'svelte';

  let data = $state<Progress | null>(null);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      data = await api.progress();
    } catch (e) {
      error = (e as Error).message;
    }
  });

  const errorsPer100 = $derived.by(() => {
    if (!data || !data.user_moves) return null;
    const errs = data.phases.reduce((s, p) => s + p.errors, 0);
    return Math.round((errs * 1000) / data.user_moves) / 10;
  });
  const estimated = $derived(data?.games.filter((g) => g.estimate != null).length ?? 0);
  const maxMotif = $derived(Math.max(0.1, ...(data?.motifs.map((m) => m.per_100_moves) ?? [])));
  const maxPhase = $derived(Math.max(0.1, ...(data?.phases.map((p) => p.per_100_moves) ?? [])));
  const PHASE_ORDER = { opening: 0, middlegame: 1, endgame: 2 } as const;
  const scoreText = (s: number | null) => (s === 1 ? 'win' : s === 0 ? 'loss' : s === 0.5 ? 'draw' : '–');
</script>

<svelte:head><title>Progress · chessgpt</title></svelte:head>

<h1>Progress</h1>

{#if error}
  <p class="error">{error}</p>
{:else if !data}
  <p class="muted">Loading…</p>
{:else if !data.games.length}
  <section class="card pad">
    <p>No analysed games with your side marked yet.</p>
    <p class="muted small">
      Import your games, mark which side you played (Lichess and Chess.com imports do this for you) and analyse them. Your
      rating trend and recurring mistakes show up here.
    </p>
    <a href="/import">Import games →</a>
  </section>
{:else}
  <div class="tiles">
    <div class="card pad tile">
      <div class="muted small">Performance rating</div>
      <div class="big">{data.performance ?? '–'}</div>
      <div class="muted small">from results against rated opponents, last 20 games</div>
    </div>
    <div class="card pad tile">
      <div class="muted small">Strength from move quality</div>
      <div class="big">
        {#if data.rolling_estimate && estimated >= 5}≈{data.rolling_estimate}<span class="margin"> ± {data.estimate_margin}</span>{:else}–{/if}
      </div>
      <div class="muted small">
        {estimated >= 5 ? `from your last ${Math.min(20, estimated)} games; firmer with more games` : 'needs at least 5 analysed games'}
      </div>
    </div>
    <div class="card pad tile">
      <div class="muted small">Mistakes and blunders</div>
      <div class="big">{errorsPer100 ?? '–'}</div>
      <div class="muted small">per 100 of your moves ({data.user_moves} moves)</div>
    </div>
  </div>

  {#if estimated >= 5}
    <section class="card pad">
      <h2>Rating and move quality over time</h2>
      <RatingTrend games={data.games} />
      <p class="muted small">
        Move quality is judged by Stockfish and averaged over up to 20 games; a single game says little. When the orange line
        runs above your rating, your play is ahead of your results.
      </p>
    </section>
  {/if}

  <div class="cols">
    <section class="card pad">
      <h2>What your mistakes are made of</h2>
      {#if !data.motifs.length}
        <p class="muted small">No tactical patterns found in your mistakes yet.</p>
      {:else}
        <ul class="bars">
          {#each data.motifs.slice(0, 8) as m (m.tag)}
            <li title="{m.missed} missed, {m.allowed} allowed">
              <span class="label">{m.label}</span>
              <span class="track"><span class="fill" style="width: {(m.per_100_moves / maxMotif) * 100}%"></span></span>
              <span class="val mono">{m.per_100_moves}</span>
              <span class="muted small detail">
                {#if m.missed}missed {m.missed}{/if}{#if m.missed && m.allowed} · {/if}{#if m.allowed}allowed {m.allowed}{/if}
              </span>
            </li>
          {/each}
        </ul>
        <p class="muted small">Per 100 of your moves. "Missed": the better move used it. "Allowed": your move let the opponent use it.</p>
      {/if}
    </section>

    <section class="card pad">
      <h2>By game phase</h2>
      <ul class="bars">
        {#each [...data.phases].sort((a, b) => PHASE_ORDER[a.phase] - PHASE_ORDER[b.phase]) as p (p.phase)}
          <li title="{p.errors} errors in {p.moves} moves">
            <span class="label cap">{p.phase}</span>
            <span class="track"><span class="fill" style="width: {(p.per_100_moves / maxPhase) * 100}%"></span></span>
            <span class="val mono">{p.per_100_moves}</span>
            <span class="muted small detail">{p.errors} of {p.moves}</span>
          </li>
        {/each}
      </ul>
      <p class="muted small">Mistakes and blunders per 100 of your moves in each phase.</p>
    </section>
  </div>

  <section class="card pad">
    <h2>Games</h2>
    <div class="scroll">
      <table>
        <thead>
          <tr><th>Date</th><th>Opponent</th><th>Result</th><th>Accuracy</th><th>Errors</th></tr>
        </thead>
        <tbody>
          {#each [...data.games].reverse() as g (g.game_id)}
            <tr>
              <td>{g.date ?? ''}</td>
              <td><a href="/analyse/{g.game_id}">{g.opponent}</a>{#if g.opponent_elo} <span class="muted">({g.opponent_elo})</span>{/if}</td>
              <td>{scoreText(g.score)}</td>
              <td class="mono">{g.accuracy != null ? `${g.accuracy.toFixed(1)}%` : '–'}</td>
              <td class="mono">{g.errors}/{g.moves}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </section>
{/if}

<style>
  .pad {
    padding: 1rem 1.2rem;
  }
  .small {
    font-size: 0.82rem;
  }
  h2 {
    font-size: 1.05rem;
    margin: 0 0 0.6rem;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
    gap: 1rem;
    margin-bottom: 1rem;
  }
  .big {
    font-size: 2rem;
    font-weight: 700;
    line-height: 1.2;
  }
  .margin {
    font-size: 1rem;
    font-weight: 400;
    color: var(--muted);
  }
  section {
    margin-bottom: 1rem;
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr));
    gap: 1rem;
  }
  .bars {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    gap: 0.45rem;
  }
  .bars li {
    display: grid;
    grid-template-columns: 8.5rem 1fr 2.5rem;
    align-items: center;
    gap: 0.5rem;
  }
  .bars .detail {
    grid-column: 2 / 4;
    margin-top: -0.3rem;
  }
  .track {
    height: 10px;
    background: var(--surface-2, var(--bg));
    border-radius: 4px;
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: #2a78d6;
    border-radius: 0 4px 4px 0;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme='light'])) .fill {
      background: #3987e5;
    }
  }
  :global(:root[data-theme='dark']) .fill {
    background: #3987e5;
  }
  .val {
    text-align: right;
  }
  .cap {
    text-transform: capitalize;
  }
  .scroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: 0.35rem 0.5rem;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
</style>
