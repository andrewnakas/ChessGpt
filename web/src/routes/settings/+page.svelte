<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import type { Meta, Provider, ProviderKind, ProviderTestResult, Settings } from '$lib/api/types';

  const app = getContext<{ refresh: () => Promise<void>; meta: Meta | null; mode: string }>('app');

  let settings = $state<Settings | null>(null);
  let lichessToken = $state('');
  let saved = $state(false);
  let settingsError = $state<string | null>(null);
  let providers = $state<Provider[]>([]);
  let tests = $state<Record<string, ProviderTestResult | 'running'>>({});
  let providerError = $state<string | null>(null);

  const KINDS: { kind: ProviderKind; label: string; model: string; base: string; key: boolean; hint: string }[] = [
    { kind: 'anthropic', label: 'Claude (Anthropic)', model: 'claude-opus-5', base: 'https://api.anthropic.com', key: true, hint: 'Key from console.anthropic.com. claude-opus-5 gives the best explanations; claude-sonnet-5 is cheaper.' },
    { kind: 'openai', label: 'OpenAI', model: 'gpt-5', base: 'https://api.openai.com/v1', key: true, hint: 'Key from platform.openai.com.' },
    { kind: 'openrouter', label: 'OpenRouter', model: 'anthropic/claude-opus-5', base: 'https://openrouter.ai/api/v1', key: true, hint: 'One key, hundreds of models. Use a model that supports tool calling.' },
    { kind: 'ollama', label: 'Ollama (local)', model: 'qwen3:8b', base: 'http://localhost:11434/v1', key: false, hint: 'Runs on your machine. Pick a model with tool support (Qwen 3, Llama 3.1+). Small models explain less well; answers are still move-checked.' },
    { kind: 'openai_compatible', label: 'Other OpenAI-compatible server', model: 'local-model', base: 'http://localhost:1234/v1', key: false, hint: 'LM Studio, llama.cpp server, vLLM, ...' }
  ];

  let form = $state({ kind: 'anthropic' as ProviderKind, model: 'claude-opus-5', base: '', key: '', label: '', makeDefault: true });
  const kindInfo = $derived(KINDS.find((k) => k.kind === form.kind)!);

  onMount(async () => {
    [settings, providers] = await Promise.all([api.settings(), api.providers()]);
  });

  function pickKind(kind: ProviderKind) {
    const k = KINDS.find((x) => x.kind === kind)!;
    form = { ...form, kind, model: k.model, base: '' };
  }

  async function saveSettings(e: Event) {
    e.preventDefault();
    if (!settings) return;
    settingsError = null;
    try {
      settings = await api.saveSettings({
        elo: settings.elo,
        lichess_username: settings.lichess_username,
        chesscom_username: settings.chesscom_username,
        explorer_enabled: settings.explorer_enabled,
        lichess_token: lichessToken.trim() ? lichessToken.trim() : null
      });
      lichessToken = '';
      saved = true;
      setTimeout(() => (saved = false), 2000);
    } catch (err) {
      settingsError = (err as Error).message;
    }
  }

  async function clearToken() {
    if (!settings) return;
    settings = await api.saveSettings({ ...settings, lichess_token: '' });
  }

  async function addProvider(e: Event) {
    e.preventDefault();
    providerError = null;
    try {
      const p = await api.createProvider({
        kind: form.kind,
        label: form.label || null,
        base_url: form.base || null,
        model: form.model || null,
        api_key: form.key || null,
        is_default: form.makeDefault
      });
      providers = await api.providers();
      form = { ...form, key: '', label: '' };
      await app.refresh();
      test(p.id);
    } catch (err) {
      providerError = (err as Error).message;
    }
  }

  async function makeDefault(p: Provider) {
    await api.updateProvider(p.id, { kind: p.kind, label: p.label, base_url: p.base_url, model: p.model, api_key: null, is_default: true });
    providers = await api.providers();
  }

  async function changeModel(p: Provider, model: string) {
    await api.updateProvider(p.id, { kind: p.kind, label: p.label, base_url: p.base_url, model, api_key: null, is_default: p.is_default });
    providers = await api.providers();
  }

  async function remove(p: Provider) {
    if (!confirm(`Remove ${p.label}?`)) return;
    await api.deleteProvider(p.id);
    providers = await api.providers();
    await app.refresh();
  }

  async function test(pid: string) {
    tests = { ...tests, [pid]: 'running' };
    try {
      tests = { ...tests, [pid]: await api.testProvider(pid) };
    } catch (err) {
      tests = { ...tests, [pid]: { ok: false, message: (err as Error).message, latency_ms: 0 } };
    }
  }
</script>

<h1>Settings</h1>

<div class="grid">
  <section class="card pad">
    <h2>You</h2>
    {#if settings}
      <form onsubmit={saveSettings}>
        <label>
          Your rating
          <input type="number" min="100" max="3500" step="50" bind:value={settings.elo} />
          <span class="muted small">Explanations are written for this level: vocabulary, depth and line length.</span>
        </label>
        <label>
          Lichess username
          <input type="text" bind:value={settings.lichess_username} placeholder="optional" />
        </label>
        <label>
          Chess.com username
          <input type="text" bind:value={settings.chesscom_username} placeholder="optional" />
        </label>
        <p class="muted small">Usernames tell chessgpt which side you played in imported games.</p>
        {#if app.mode !== 'browser'}
        <label>
          Lichess API token {#if settings.has_lichess_token}<span class="chip ok">stored</span>{/if}
          <input type="password" bind:value={lichessToken} placeholder={settings.has_lichess_token ? 'leave empty to keep' : 'lip_...'} autocomplete="off" />
          <span class="muted small">
            Needed for importing by username and for the opening explorer. Create one at
            <a href="https://lichess.org/account/oauth/token" target="_blank" rel="noreferrer">lichess.org/account/oauth/token</a>
            with no scopes. Stored encrypted.
            {#if settings.has_lichess_token}<button type="button" class="ghost small danger" onclick={clearToken}>Remove</button>{/if}
          </span>
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={settings.explorer_enabled} />
          Let the coach look up openings in the Lichess explorer
        </label>
        {/if}
        {#if settingsError}<p class="error">{settingsError}</p>{/if}
        <div><button class="primary" type="submit">Save</button> {#if saved}<span class="ok">Saved</span>{/if}</div>
      </form>
    {/if}
  </section>

  <section class="card pad">
    <h2>AI coach</h2>
    {#if app.mode === 'browser'}
      <p class="muted">The coach runs on the chessgpt server, which is offline right now. Games you analyse here are saved in this browser.</p>
    {:else if app.meta?.managed_provider}
      <p>This site provides the AI coach: <b>{app.meta.provider_label}</b>. You don't need a key.</p>
      {#if app.meta.budget_used != null}
        <p class="muted small">Today's shared coach budget used: {Math.round(app.meta.budget_used * 100)}%.</p>
      {/if}
      <p class="muted small">You can also use chessgpt from your own Claude or ChatGPT: <a href="/connect">connect it</a>.</p>
    {:else}
    <p class="muted small">
      The coach needs a language model. Your key stays on this server, encrypted, and is only sent to the provider you choose.
    </p>
    {#each providers as p (p.id)}
      {@const t = tests[p.id]}
      <div class="prov">
        <div class="row">
          <b>{p.label}</b>
          {#if p.is_default}<span class="chip">default</span>{:else}<button class="ghost small" onclick={() => makeDefault(p)}>Make default</button>{/if}
          <span class="spacer"></span>
          <button class="small" onclick={() => test(p.id)} disabled={t === 'running'}>{t === 'running' ? 'Testing…' : 'Test'}</button>
          <button class="ghost small danger" onclick={() => remove(p)}>Remove</button>
        </div>
        <div class="row small">
          <span class="muted">Model</span>
          <input
            type="text"
            class="mono model"
            value={p.model}
            onchange={(e) => changeModel(p, (e.currentTarget as HTMLInputElement).value)}
          />
          <span class="muted mono url">{p.base_url}</span>
        </div>
        {#if t && t !== 'running'}
          <div class="small {t.ok ? 'ok' : 'error'}">{t.ok ? '✓' : '✗'} {t.message} {#if t.ok}({t.latency_ms} ms){/if}</div>
        {/if}
      </div>
    {/each}

    <form class="add" onsubmit={addProvider}>
      <h3>Add a provider</h3>
      <div class="kinds">
        {#each KINDS as k}
          <button type="button" class:on={form.kind === k.kind} onclick={() => pickKind(k.kind)}>{k.label}</button>
        {/each}
      </div>
      <p class="muted small">{kindInfo.hint}</p>
      <label>Model <input type="text" class="mono" bind:value={form.model} /></label>
      {#if kindInfo.key}
        <label>API key <input type="password" bind:value={form.key} autocomplete="off" required /></label>
      {/if}
      <label>
        Base URL <input type="url" class="mono" bind:value={form.base} placeholder={kindInfo.base} />
      </label>
      <label class="check"><input type="checkbox" bind:checked={form.makeDefault} /> Use as default</label>
      {#if providerError}<p class="error">{providerError}</p>{/if}
      <div><button class="primary" type="submit">Add provider</button></div>
    </form>
    {/if}
  </section>
</div>

<style>
  h1 {
    font-size: 1.5rem;
    margin: 0 0 0.8rem;
  }
  h2 {
    margin: 0 0 0.6rem;
    font-size: 1.15rem;
  }
  h3 {
    margin: 0.6rem 0 0.2rem;
    font-size: 1rem;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(300px, 1fr) minmax(340px, 1.3fr);
    gap: 1.2rem;
    align-items: start;
  }
  .pad {
    padding: 1rem 1.2rem;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-weight: 500;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
    font-weight: 400;
  }
  .small {
    font-size: 0.82rem;
    font-weight: 400;
  }
  .ok {
    color: var(--ok);
  }
  .chip.ok {
    color: var(--ok);
  }
  .prov {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.6rem 0.7rem;
    margin-bottom: 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .model {
    width: 16rem;
    padding: 0.2rem 0.4rem;
  }
  .url {
    font-size: 0.75rem;
  }
  .add {
    border-top: 1px solid var(--border);
    margin-top: 0.8rem;
    padding-top: 0.4rem;
  }
  .kinds {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .kinds button.on {
    background: var(--accent);
    color: var(--accent-contrast);
    border-color: var(--accent);
  }
  @media (max-width: 900px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
