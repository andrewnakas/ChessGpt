<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import type { DrillOverview, DrillSource, TechniqueMastery } from '$lib/api/types';
  import { tagLabel } from '$lib/chess';

  let data = $state<DrillOverview | null>(null);
  let error = $state<string | null>(null);
  let building = $state<string | null>(null);
  let pro = $state(readPro());

  function readPro(): boolean {
    try {
      return localStorage.getItem('drills.pro') === '1';
    } catch {
      return false;
    }
  }
  function setPro(on: boolean) {
    pro = on;
    try {
      localStorage.setItem('drills.pro', on ? '1' : '0');
    } catch {
      /* private mode: the toggle lasts for this page only */
    }
  }

  onMount(async () => {
    try {
      data = await api.drills();
    } catch (e) {
      error = (e as Error).message;
    }
  });

  async function start(source: DrillSource, key: string) {
    building = key;
    error = null;
    try {
      const set = await api.createDrill(source);
      await goto(`/drills/${set.id}`);
    } catch (e) {
      error = (e as Error).message;
      building = null;
    }
  }

  const stat = (t: TechniqueMastery) =>
    t.attempted
      ? `${t.solved}/${t.attempted} solved · ${t.clean} clean${t.median_ms ? ` · ~${Math.round(t.median_ms / 1000)}s` : ''}`
      : 'not drilled yet';
  const pct = (t: TechniqueMastery) => (t.attempted ? Math.round((100 * t.clean) / t.attempted) : 0);
</script>

<svelte:head><title>Drills · chessgpt</title></svelte:head>

<h1>Drills</h1>
<p class="muted">
  One idea, trained four ways: spot it, find it in fresh positions (some of them your own, redrawn), stop it when it's
  aimed at you, and use it. Every position is checked by Stockfish. Misses go to your <a href="/train">Train</a> queue.
</p>

{#if error}<p class="error">{error}</p>{/if}

{#if !data && !error}
  <p class="muted">Loading…</p>
{:else if data}
  <section class="card pad focus">
    <div>
      <h2>Your focus</h2>
      <p class="muted small">
        {#if data.focus.length}
          The ideas behind most of your mistakes: {data.focus.slice(0, 2).map(tagLabel).join(' and ')}.
        {/if}
        A set is about 9 positions and takes 5–10 minutes. This week: <b>{data.week_done}/{data.week_goal}</b> sets.
      </p>
      <label class="small pro">
        <input type="checkbox" checked={pro} onchange={(e) => setPro(e.currentTarget.checked)} />
        Pro mode: no hints, timed
      </label>
    </div>
    <div class="row">
      {#each data.focus.slice(0, 2) as tag}
        <button class="primary" disabled={!!building} onclick={() => start({ kind: 'theme', tag }, tag)}>
          {building === tag ? 'Building…' : `Drill ${tagLabel(tag).toLowerCase()}`}
        </button>
      {/each}
    </div>
  </section>

  <h2 class="section">All techniques</h2>
  <div class="grid">
    {#each data.techniques as t}
      <button class="card tile" disabled={!!building} onclick={() => start({ kind: 'theme', tag: t.tag }, t.tag)}>
        <span class="name">{building === t.tag ? 'Building…' : t.label}</span>
        <span class="bar"><span style="width: {pct(t)}%"></span></span>
        <span class="muted small">{stat(t)}</span>
      </button>
    {/each}
  </div>

  {#if data.recent.length}
    <h2 class="section">Recent sets</h2>
    <ul class="recent">
      {#each data.recent as s}
        <li>
          <a href="/drills/{s.id}">{s.label}</a>
          <span class="muted small">
            {s.solved}/{s.items} solved{#if !s.completed_at} · {s.items - s.attempted} left{/if} ·
            {new Date(s.created_at).toLocaleDateString()}
          </span>
        </li>
      {/each}
    </ul>
  {/if}
{/if}

<style>
  .pad {
    padding: 1rem 1.2rem;
  }
  .small {
    font-size: 0.85rem;
  }
  h2 {
    font-size: 1.05rem;
    margin: 0 0 0.4rem;
  }
  .section {
    margin-top: 1.4rem;
  }
  .focus {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    justify-content: space-between;
    align-items: center;
  }
  .pro {
    display: inline-flex;
    gap: 0.4rem;
    align-items: center;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr));
    gap: 0.7rem;
  }
  .tile {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0.35rem;
    padding: 0.8rem 0.9rem;
    text-align: left;
    cursor: pointer;
    font: inherit;
    color: inherit;
  }
  .tile:hover:not(:disabled) {
    border-color: var(--ok);
  }
  .name {
    font-weight: 600;
  }
  .bar {
    height: 4px;
    border-radius: 2px;
    background: var(--border);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--ok);
  }
  .recent {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .recent li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    padding: 0.35rem 0;
    border-bottom: 1px solid var(--border);
  }
</style>
