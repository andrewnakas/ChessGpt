<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { api, type Account } from '$lib/api/client';

  const app = getContext<{ account: Account | null; accounts: boolean }>('app');
  const mcpUrl = $derived(typeof location !== 'undefined' ? `${location.origin}/mcp` : '/mcp');
  let copied = $state(false);
  let connections = $state<{ client_id: string; name: string; last_used: number }[]>([]);

  onMount(async () => {
    if (!app.accounts || app.account) {
      try {
        connections = await api.connections();
      } catch {
        /* not signed in */
      }
    }
  });

  async function copy() {
    await navigator.clipboard.writeText(mcpUrl);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  async function disconnect(id: string) {
    await api.disconnect(id);
    connections = connections.filter((c) => c.client_id !== id);
  }

  const examples = [
    'Analyse my last Lichess game and explain my two biggest mistakes.',
    'Here is a PGN, what was the turning point? [paste]',
    'What should White play here? r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3',
    'Based on my games on chessgpt, what should I study this week?'
  ];
</script>

<article>
  <h1>Use chessgpt inside Claude or ChatGPT</h1>
  <p class="lead">
    Add chessgpt to the assistant you already pay for. Your Claude or ChatGPT does the talking; chessgpt gives it
    Stockfish, move checking, and your game library. No API key, and everything it analyses is saved to your account
    here, ready to replay on the full board.
  </p>

  <div class="url card">
    <span class="muted">Connector URL</span>
    <code>{mcpUrl}</code>
    <button class="primary" onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
  </div>

  <div class="grid">
    <section class="card">
      <h2>Claude</h2>
      <p class="muted small">Free (one custom connector), Pro, Max, Team and Enterprise; web, desktop and mobile.</p>
      <ol>
        <li>In Claude open <b>Settings → Connectors</b> (on Team/Enterprise an owner adds it under Organization settings).</li>
        <li>Choose <b>Add custom connector</b>, name it <i>chessgpt</i>, and paste the URL above.</li>
        <li>Click <b>Connect</b>. You'll sign in to chessgpt and approve access.</li>
        <li>In a chat, turn chessgpt on from the tools menu and ask away.</li>
      </ol>
    </section>
    <section class="card">
      <h2>ChatGPT</h2>
      <p class="muted small">Plus, Pro, Business, Enterprise and Education on the web.</p>
      <ol>
        <li>Open <b>Settings → Apps</b> (turn on <b>Developer mode</b> under Security if custom apps are hidden).</li>
        <li>Choose <b>Create app</b> / <b>Add connector</b>, name it <i>chessgpt</i>, paste the URL, authentication <b>OAuth</b>.</li>
        <li>Sign in to chessgpt and approve access.</li>
        <li>Start a chat, pick chessgpt from the <b>+</b> menu, and ask.</li>
      </ol>
    </section>
  </div>

  <section class="card">
    <h2>Try asking</h2>
    <ul class="ex">
      {#each examples as e}<li>“{e}”</li>{/each}
    </ul>
    <p class="muted small">
      Answers come with an interactive board. Every game review includes a link back here, where the same game is in
      your library with the eval graph, move-by-move classification and the coach.
    </p>
  </section>

  <section class="card">
    <h2>What the assistant can do</h2>
    <ul>
      <li><b>analyze_position</b>: Stockfish's best lines for any position</li>
      <li><b>check_line</b>: plays out a line, confirms every move is legal, and evaluates the result</li>
      <li><b>analyze_game</b>: saves a game to your library and classifies every move the way Lichess does</li>
      <li><b>game_report</b>, <b>my_games</b>, <b>my_weaknesses</b>: your library and your recurring mistakes</li>
      <li><b>import_games</b>: pulls recent games from Lichess or Chess.com</li>
      <li><b>opening_explorer</b>: opening statistics (when you signed in with Lichess)</li>
    </ul>
    <p class="muted small">It can't see your password or change anything outside your chessgpt library.</p>
  </section>

  {#if connections.length}
    <section class="card">
      <h2>Connected apps</h2>
      {#each connections as c (c.client_id)}
        <div class="conn">
          <b>{c.name}</b>
          <span class="muted small">last connected {new Date(c.last_used).toLocaleDateString()}</span>
          <button class="ghost danger" onclick={() => disconnect(c.client_id)}>Disconnect</button>
        </div>
      {/each}
    </section>
  {/if}
</article>

<style>
  article {
    max-width: 60rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  h1 {
    margin: 0;
    font-size: 1.6rem;
  }
  h2 {
    margin: 0 0 0.4rem;
    font-size: 1.1rem;
  }
  .lead {
    margin: 0;
    font-size: 1.05rem;
    max-width: 48rem;
  }
  .card {
    padding: 1rem 1.2rem;
  }
  .url {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    flex-wrap: wrap;
  }
  .url code {
    font-size: 1.05rem;
    flex: 1;
    word-break: break-all;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }
  ol,
  ul {
    margin: 0.3rem 0;
    padding-left: 1.3rem;
  }
  li {
    margin: 0.25rem 0;
  }
  .ex li {
    font-style: italic;
  }
  .small {
    font-size: 0.85rem;
  }
  .conn {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.3rem 0;
  }
  .conn button {
    margin-left: auto;
  }
  @media (max-width: 800px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
