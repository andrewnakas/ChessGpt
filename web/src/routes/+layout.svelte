<script lang="ts">
  import '../app.css';
  import { page } from '$app/state';
  import { api, authListeners } from '$lib/api/client';
  import type { Meta } from '$lib/api/types';
  import { onMount, setContext } from 'svelte';

  let { children } = $props();
  let meta = $state<Meta | null>(null);
  let needLogin = $state(false);
  let password = $state('');
  let loginError = $state<string | null>(null);
  let ready = $state(false);

  const app = $state({ meta: null as Meta | null, refresh: async () => {} });
  setContext('app', app);

  async function loadMeta() {
    try {
      meta = await api.meta();
      app.meta = meta;
    } catch {
      /* 401 handled by listener */
    }
  }
  app.refresh = loadMeta;

  onMount(() => {
    const onAuth = () => (needLogin = true);
    authListeners.add(onAuth);
    api
      .session()
      .then(async (s) => {
        needLogin = s.required && !s.authenticated;
        if (!needLogin) await loadMeta();
      })
      .finally(() => (ready = true));
    return () => authListeners.delete(onAuth);
  });

  async function login(e: Event) {
    e.preventDefault();
    loginError = null;
    try {
      await api.login(password);
      needLogin = false;
      password = '';
      await loadMeta();
      location.reload();
    } catch (err) {
      loginError = (err as Error).message;
    }
  }

  const nav = [
    { href: '/', label: 'Games' },
    { href: '/import', label: 'Import' },
    { href: '/board', label: 'Board' },
    { href: '/settings', label: 'Settings' }
  ];
  const active = (href: string) =>
    href === '/' ? page.url.pathname === '/' || page.url.pathname.startsWith('/analyse') : page.url.pathname.startsWith(href);
</script>

<header>
  <a class="brand" href="/">
    <img src="/favicon.svg" alt="" width="26" height="26" />
    <span>chess<b>gpt</b></span>
  </a>
  <nav>
    {#each nav as n}
      <a href={n.href} class:active={active(n.href)}>{n.label}</a>
    {/each}
  </nav>
  <div class="right muted">
    {#if meta}
      <span title="{meta.engine_workers} engine workers × {meta.engine_threads} threads">{meta.engine}</span>
      {#if !meta.has_provider}<a class="warn" href="/settings">No AI provider</a>{/if}
    {/if}
  </div>
</header>

<main>
  {#if needLogin}
    <form class="login card" onsubmit={login}>
      <h2>chessgpt</h2>
      <p class="muted">This server is private. Enter the password.</p>
      <input type="password" bind:value={password} placeholder="Password" autocomplete="current-password" />
      {#if loginError}<p class="error">{loginError}</p>{/if}
      <button class="primary" type="submit">Sign in</button>
    </form>
  {:else if ready}
    {@render children()}
  {/if}
</main>

<footer class="muted">
  <a href="https://github.com/chessgpt/chessgpt">chessgpt</a> is open source (AGPL-3.0). Analysis by
  <a href="https://stockfishchess.org">Stockfish</a> (GPL-3.0). Openings: lichess-org/chess-openings (CC0).
</footer>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 1.5rem;
    padding: 0.55rem 1.2rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    position: sticky;
    top: 0;
    z-index: 10;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--text);
    font-size: 1.2rem;
    letter-spacing: -0.02em;
    text-decoration: none;
  }
  .brand b {
    color: var(--accent);
  }
  nav {
    display: flex;
    gap: 0.2rem;
  }
  nav a {
    color: var(--muted);
    padding: 0.3rem 0.7rem;
    border-radius: 8px;
    text-decoration: none;
    font-weight: 500;
  }
  nav a:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  nav a.active {
    color: var(--text);
    background: var(--surface-2);
  }
  .right {
    margin-left: auto;
    display: flex;
    gap: 0.8rem;
    font-size: 0.85rem;
  }
  .warn {
    color: var(--warn);
  }
  main {
    padding: 1rem 1.2rem;
    max-width: 1500px;
    margin: 0 auto;
    min-height: calc(100vh - 110px);
  }
  footer {
    text-align: center;
    font-size: 0.75rem;
    padding: 1rem;
  }
  .login {
    max-width: 360px;
    margin: 10vh auto;
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .login h2 {
    margin: 0;
  }
  @media (max-width: 640px) {
    header {
      flex-wrap: wrap;
      gap: 0.6rem;
    }
    .right {
      display: none;
    }
  }
</style>
