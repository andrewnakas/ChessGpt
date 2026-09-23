<script lang="ts">
  import { goto } from '$app/navigation';
  import { api } from '$lib/api/client';
  import type { ImportRequest, ImportResponse, Settings } from '$lib/api/types';
  import { isValidFen } from '$lib/chess';
  import { onMount } from 'svelte';

  type Tab = 'paste' | 'lichess' | 'chesscom';
  let tab = $state<Tab>('paste');
  let text = $state('');
  let username = $state('');
  let max = $state(20);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let result = $state<ImportResponse | null>(null);
  let settings = $state<Settings | null>(null);

  onMount(async () => {
    settings = await api.settings();
    username = settings.lichess_username ?? '';
  });

  $effect(() => {
    if (!settings) return;
    if (tab === 'lichess') username = settings.lichess_username ?? '';
    if (tab === 'chesscom') username = settings.chesscom_username ?? '';
  });

  function detect(t: string): ImportRequest {
    const s = t.trim();
    if (/lichess\.org\/[A-Za-z0-9]{8}/.test(s) && !s.includes('[')) return { source: 'lichess_game', id: s };
    if (!s.includes('\n') && isValidFen(s)) return { source: 'fen', fen: s };
    return { source: 'pgn', pgn: s };
  }

  async function submit(e: Event) {
    e.preventDefault();
    error = null;
    result = null;
    busy = true;
    try {
      let req: ImportRequest;
      if (tab === 'paste') {
        if (!text.trim()) throw new Error('Paste a PGN, a FEN, or a Lichess game link.');
        req = detect(text);
        if (req.source === 'fen') {
          goto(`/board?fen=${encodeURIComponent((req as { fen: string }).fen)}`);
          return;
        }
      } else if (tab === 'lichess') {
        req = { source: 'lichess', username: username.trim(), max };
      } else {
        req = { source: 'chesscom', username: username.trim(), max };
      }
      result = await api.importGames(req);
      if (result.games.length === 1) goto(`/analyse/${result.games[0].id}`);
    } catch (err) {
      error = (err as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<h1>Import</h1>

<div class="tabs">
  <button class:on={tab === 'paste'} onclick={() => (tab = 'paste')}>Paste PGN / FEN / link</button>
  <button class:on={tab === 'lichess'} onclick={() => (tab = 'lichess')}>Lichess</button>
  <button class:on={tab === 'chesscom'} onclick={() => (tab = 'chesscom')}>Chess.com</button>
</div>

<form class="card box" onsubmit={submit}>
  {#if tab === 'paste'}
    <label for="pgn">PGN (one or many games), a FEN, or a lichess.org game link</label>
    <textarea id="pgn" rows="12" class="mono" bind:value={text} placeholder={'[Event "Casual game"]\n\n1. e4 e5 2. Nf3 Nc6 3. Bb5 a6 ...'}></textarea>
  {:else}
    <div class="row">
      <label>
        Username
        <input type="text" bind:value={username} placeholder={tab === 'lichess' ? 'Lichess username' : 'Chess.com username'} />
      </label>
      <label class="narrow">
        Recent games
        <input type="number" min="1" max="300" bind:value={max} />
      </label>
    </div>
    {#if tab === 'lichess' && settings && !settings.has_lichess_token}
      <p class="note">
        Lichess currently only exports a player's game list to signed-in API clients. Add a Lichess personal API token
        (no scopes needed) in <a href="/settings">Settings</a>, or paste single game links in the first tab.
      </p>
    {/if}
    {#if tab === 'chesscom'}
      <p class="muted small">Chess.com archives are fetched one month at a time, newest first.</p>
    {/if}
  {/if}
  {#if error}<p class="error">{error}</p>{/if}
  <div>
    <button class="primary" type="submit" disabled={busy}>
      {#if busy}<span class="spinner"></span> Importing…{:else}Import{/if}
    </button>
  </div>
</form>

{#if result}
  <div class="card box">
    <p>
      Imported <b>{result.games.length - result.duplicates}</b> new game(s){#if result.duplicates}, {result.duplicates} already in your library{/if}.
      {#if result.errors.length}<span class="error">{result.errors.length} could not be read.</span>{/if}
    </p>
    <ul>
      {#each result.games.slice(0, 50) as g (g.id)}
        <li><a href="/analyse/{g.id}">{g.white} vs {g.black}</a> <span class="muted">{g.result} · {g.opening ?? ''}</span></li>
      {/each}
    </ul>
    <a href="/">All games →</a>
  </div>
{/if}

<style>
  h1 {
    font-size: 1.5rem;
    margin: 0 0 0.8rem;
  }
  .tabs {
    display: flex;
    gap: 0.3rem;
    margin-bottom: 0.6rem;
  }
  .tabs button.on {
    background: var(--accent);
    color: var(--accent-contrast);
    border-color: var(--accent);
  }
  .box {
    padding: 1rem 1.2rem;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
    max-width: 52rem;
    margin-bottom: 1rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-weight: 500;
  }
  .row {
    display: flex;
    gap: 1rem;
  }
  .row label {
    flex: 1;
  }
  .row .narrow {
    flex: 0 0 9rem;
  }
  .note {
    border-left: 3px solid var(--warn);
    padding: 0.4rem 0.7rem;
    background: var(--surface-2);
    border-radius: 0 8px 8px 0;
    margin: 0;
  }
  .small {
    font-size: 0.85rem;
  }
</style>
