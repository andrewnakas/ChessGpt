<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { getContext } from 'svelte';
  import { api } from '$lib/api/client';

  const app = getContext<{ refresh: () => Promise<void> }>('app');

  const ret = $derived.by(() => {
    const r = page.url.searchParams.get('return') ?? '/';
    return r.startsWith('/') && !r.startsWith('//') ? r : '/';
  });
  let mode = $state<'login' | 'register'>('login');
  let email = $state('');
  let password = $state('');
  let name = $state('');
  let busy = $state(false);
  let error = $state<string | null>(page.url.searchParams.get('error'));

  async function submit(e: Event) {
    e.preventDefault();
    error = null;
    busy = true;
    try {
      if (mode === 'login') await api.login(email, password);
      else await api.register(email, password, name || undefined);
      await app.refresh();
      // Full navigation so server-side OAuth pages (connecting Claude/ChatGPT) see the new cookie.
      if (ret.startsWith('/oauth/')) location.href = ret;
      else goto(ret);
    } catch (err) {
      error = (err as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<div class="wrap">
  <div class="card box">
    <h1>{mode === 'login' ? 'Sign in to chessgpt' : 'Create your account'}</h1>
    <a class="lichess" href="/auth/lichess?return={encodeURIComponent(ret)}">
      <svg viewBox="0 0 50 50" width="20" height="20" aria-hidden="true"><path fill="currentColor" d="M38.956.5c-3.53.418-6.452.902-9.286 2.984C5.534 1.786-.692 18.533.68 29.364 3.493 50.214 31.918 55.785 41.329 41.7c-7.444 7.696-19.276 8.752-28.323 3.084C3.959 39.116-.506 27.392 4.683 17.567 9.873 7.742 18.996 4.535 29.03 6.405c2.43-1.418 5.225-3.22 7.655-3.187l-1.694 4.86 12.752 21.37c-.439 5.654-5.459 6.112-5.459 6.112-.574-1.47-1.634-2.942-4.842-6.036-3.207-3.094-17.465-10.177-15.788-16.207-2.001 6.967 10.311 14.152 14.04 17.663 3.73 3.51 5.426 6.04 5.795 6.756 0 0 9.392-2.504 7.838-8.927L37.4 7.171z"/></svg>
      Continue with Lichess
    </a>
    <p class="muted small center">Imports your Lichess games and unlocks the opening explorer.</p>
    <div class="or"><span>or</span></div>
    <form onsubmit={submit}>
      {#if mode === 'register'}
        <label>Name <input type="text" bind:value={name} autocomplete="nickname" placeholder="optional" /></label>
      {/if}
      <label>Email <input type="text" bind:value={email} autocomplete="email" required /></label>
      <label>
        Password
        <input
          type="password"
          bind:value={password}
          autocomplete={mode === 'login' ? 'current-password' : 'new-password'}
          minlength={mode === 'register' ? 8 : undefined}
          required
        />
      </label>
      {#if error}<p class="error">{error}</p>{/if}
      <button class="primary" type="submit" disabled={busy}>{mode === 'login' ? 'Sign in' : 'Create account'}</button>
    </form>
    <p class="muted small center">
      {#if mode === 'login'}
        New here? <button class="linkish" onclick={() => (mode = 'register')}>Create an account</button>
      {:else}
        Have an account? <button class="linkish" onclick={() => (mode = 'login')}>Sign in</button>
      {/if}
    </p>
  </div>
</div>

<style>
  .wrap {
    display: flex;
    justify-content: center;
    padding-top: 6vh;
  }
  .box {
    width: 100%;
    max-width: 380px;
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  h1 {
    margin: 0 0 0.3rem;
    font-size: 1.3rem;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-weight: 500;
  }
  .lichess {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 0.55rem;
    border-radius: 8px;
    background: #2a2a2a;
    color: #fff;
    font-weight: 600;
    text-decoration: none;
  }
  .lichess:hover {
    background: #3a3a3a;
    text-decoration: none;
  }
  .or {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    color: var(--muted);
    font-size: 0.8rem;
  }
  .or::before,
  .or::after {
    content: '';
    flex: 1;
    border-top: 1px solid var(--border);
  }
  .small {
    font-size: 0.82rem;
    margin: 0;
  }
  .center {
    text-align: center;
  }
  .linkish {
    background: none;
    border: none;
    padding: 0;
    color: var(--link);
  }
</style>
