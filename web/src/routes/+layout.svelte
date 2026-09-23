<script lang="ts">
  import '../app.css';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { api, authListeners, mode, type Account, type Mode } from '$lib/api/client';
  import type { Meta } from '$lib/api/types';
  import { coachAvailable, device } from '$lib/llm/device.svelte';
  import { onMount, setContext } from 'svelte';

  let { children } = $props();
  let meta = $state<Meta | null>(null);
  let ready = $state(false);

  const app = $state({
    meta: null as Meta | null,
    account: null as Account | null,
    accounts: false,
    mode: 'server' as Mode,
    refresh: async () => {}
  });
  setContext('app', app);

  // Pages anyone may open without signing in (hosted mode).
  const PUBLIC = ['/login', '/share/', '/about', '/connect', '/oauth/'];
  const isPublic = (p: string) => PUBLIC.some((x) => p === x || p.startsWith(x));

  async function loadMeta() {
    try {
      meta = await api.meta();
      app.meta = meta;
      void device.resume(meta.device_model);
    } catch {
      /* 401 handled by listener */
    }
  }
  app.refresh = async () => {
    app.mode = await mode();
    const s = await api.session();
    app.accounts = s.accounts;
    app.account = s.account;
    if (!s.accounts || s.account) await loadMeta();
  };

  function toLogin() {
    const p = page.url.pathname;
    if (!isPublic(p)) goto(`/login?return=${encodeURIComponent(p + page.url.search)}`);
  }

  onMount(() => {
    authListeners.add(toLogin);
    app
      .refresh()
      .then(() => {
        if (app.accounts && !app.account) toLogin();
      })
      .finally(() => (ready = true));
    return () => authListeners.delete(toLogin);
  });

  async function logout() {
    await api.logout();
    app.account = null;
    meta = null;
    goto('/login');
  }

  const nav = [
    { href: '/', label: 'Games' },
    { href: '/import', label: 'Import' },
    { href: '/board', label: 'Board' },
    { href: '/progress', label: 'Progress' },
    { href: '/train', label: 'Train' },
    { href: '/play', label: 'Play' },
    { href: '/connect', label: 'Claude & ChatGPT' },
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
      {#if !coachAvailable(meta.has_provider)}<a class="warn" href="/settings">{meta.device_model ? 'Turn on the coach' : 'No AI provider'}</a>{/if}
    {/if}
    {#if app.account}
      <span class="who">{app.account.display_name}</span>
      <button class="ghost small" onclick={logout}>Sign out</button>
    {:else if app.accounts}
      <a href="/login">Sign in</a>
    {/if}
  </div>
</header>

{#if app.mode === 'browser'}
  <div class="offline">
    <b>Browser mode.</b> The chessgpt server is offline, so Stockfish is running in your browser and games are saved on
    this device. The coach, accounts and the Claude/ChatGPT connector will be back when the server is.
  </div>
{/if}

<main>
  {#if ready}
    {@render children()}
  {/if}
</main>

<footer class="muted">
  <a href="https://github.com/andrewnakas/ChessGpt">chessgpt</a> is open source (AGPL-3.0). Analysis by
  <a href="https://stockfishchess.org">Stockfish</a> (GPL-3.0). <a href="/about">About, source and licenses</a>.
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
  .offline {
    background: color-mix(in srgb, var(--warn) 18%, var(--surface));
    border-bottom: 1px solid var(--border);
    padding: 0.45rem 1.2rem;
    font-size: 0.88rem;
    text-align: center;
  }
  .who {
    color: var(--text);
    font-weight: 600;
  }
  .small {
    font-size: 0.8rem;
    padding: 0.15rem 0.5rem;
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
