<script lang="ts">
  import type { DrawShape } from '@lichess-org/chessground/draw';
  import type { Key } from '@lichess-org/chessground/types';
  import { api, streams } from '$lib/api/client';
  import type { ChatMessage, ChatThread, ToolCallView, Verification } from '$lib/api/types';
  import { renderMarkdown } from '$lib/markdown';
  import { getContext } from 'svelte';
  const appCtx = getContext<{ mode?: string } | undefined>('app');

  interface Props {
    gameId?: string | null;
    fen: string;
    ply?: number | null;
    movePath: string[];
    hasProvider: boolean;
    onshapes: (shapes: DrawShape[]) => void;
  }
  let { gameId = null, fen, ply = null, movePath, hasProvider, onshapes }: Props = $props();

  let threads = $state<ChatThread[]>([]);
  let threadId = $state<string | null>(null);
  let messages = $state<ChatMessage[]>([]);
  let input = $state('');
  let busy = $state(false);
  let streaming = $state('');
  let thinking = $state('');
  let liveCalls = $state<ToolCallView[]>([]);
  let pendingCalls = $state<{ id: string; name: string }[]>([]);
  let liveVerification = $state<Verification | null>(null);
  let error = $state<string | null>(null);
  let ctrl: AbortController | null = null;
  let scroller: HTMLDivElement;

  const TOOL_LABEL: Record<string, string> = {
    analyse_position: 'Stockfish',
    play_line: 'Checking line',
    legal_moves: 'Legal moves',
    opening_lookup: 'Opening explorer',
    game_context: 'Reading the game'
  };

  $effect(() => {
    const g = gameId;
    api
      .threads(g ?? undefined)
      .then(async (t) => {
        threads = g ? t : t.filter((x) => !x.game_id);
        if (threads.length) await open(threads[0].id);
        else {
          threadId = null;
          messages = [];
        }
      })
      .catch((e) => (error = String(e.message ?? e)));
  });

  async function open(id: string) {
    threadId = id;
    const d = await api.thread(id);
    messages = d.messages;
    scrollDown();
  }

  function newThread() {
    threadId = null;
    messages = [];
    error = null;
  }

  function scrollDown() {
    queueMicrotask(() => scroller?.scrollTo({ top: scroller.scrollHeight, behavior: 'smooth' }));
  }

  function shapesFor(call: ToolCallView): DrawShape[] {
    const data = call.data as Record<string, unknown> | null | undefined;
    if (!data) return [];
    const lines = (data.lines as { uci: string[] }[] | undefined) ?? [];
    if (lines.length) {
      return lines
        .filter((l) => l.uci?.length)
        .map((l, i) => ({
          orig: l.uci[0].slice(0, 2) as Key,
          dest: l.uci[0].slice(2, 4) as Key,
          brush: i === 0 ? 'green' : 'paleBlue'
        }));
    }
    const uci = data.uci as string[] | undefined;
    if (uci?.length) return [{ orig: uci[0].slice(0, 2) as Key, dest: uci[0].slice(2, 4) as Key, brush: 'yellow' }];
    return [];
  }

  async function send(text?: string) {
    const q = (text ?? input).trim();
    if (!q || busy) return;
    error = null;
    busy = true;
    streaming = '';
    thinking = '';
    liveCalls = [];
    pendingCalls = [];
    liveVerification = null;
    input = '';
    try {
      if (!threadId) {
        const t = await api.createThread({ game_id: gameId ?? null, title: null });
        threads = [t, ...threads];
        threadId = t.id;
      }
      ctrl = new AbortController();
      await streams.chat(
        threadId,
        { text: q, fen, ply: ply ?? null, move_path: movePath, elo: null },
        (e) => {
          switch (e.type) {
            case 'user_message':
              messages = [...messages, e.message];
              break;
            case 'text_delta':
              streaming += e.text;
              thinking = '';
              break;
            case 'thinking':
              thinking = (thinking + e.text).slice(-400);
              break;
            case 'tool_call':
              pendingCalls = [...pendingCalls, { id: e.id, name: e.name }];
              break;
            case 'tool_result':
              pendingCalls = pendingCalls.filter((c) => c.id !== e.call.id);
              liveCalls = [...liveCalls, e.call];
              break;
            case 'verification':
              liveVerification = e.verification;
              break;
            case 'done':
              messages = [...messages, e.message];
              streaming = '';
              liveCalls = [];
              api.threads(gameId ?? undefined).then((t) => (threads = gameId ? t : t.filter((x) => !x.game_id)));
              break;
            case 'error':
              error = e.message;
              break;
          }
          scrollDown();
        },
        ctrl.signal
      );
    } catch (e) {
      if ((e as Error).name !== 'AbortError') error = (e as Error).message;
    } finally {
      busy = false;
      ctrl = null;
      thinking = '';
      pendingCalls = [];
    }
  }

  function stop() {
    ctrl?.abort();
  }

  const suggestions = $derived(
    gameId
      ? ['Why was my last move bad?', 'What is the plan here?', 'Where did I go wrong in this game?']
      : ['What is the best move here and why?', 'What are the key ideas in this position?', 'Is this position winning?']
  );
</script>

{#snippet calls(list: ToolCallView[])}
  {#if list.length}
    <div class="calls">
      {#each list as c (c.id)}
        <button
          class="chip call"
          class:err={c.is_error}
          title={JSON.stringify(c.input)}
          onclick={() => onshapes(shapesFor(c))}
        >
          {TOOL_LABEL[c.name] ?? c.name}: {c.summary}
        </button>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet badge(v: Verification | null | undefined)}
  {#if v}
    {#if v.status === 'ok' && !v.unverified_moves.length}
      <span class="verified" title="Every move mentioned appears in the engine results or is legal and was checked">✓ moves checked</span>
    {:else if v.status === 'ok'}
      <span class="partial" title="Legal, but not analysed by the engine: {v.unverified_moves.join(', ')}">✓ legal · some moves not engine-checked</span>
    {:else}
      <span class="partial bad" title={v.issues.join('\n')}>⚠ contains moves the engine could not confirm</span>
    {/if}
  {/if}
{/snippet}

<div class="chat">
  <div class="bar">
    <select
      value={threadId ?? ''}
      onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        if (v) open(v);
        else newThread();
      }}
    >
      <option value="">New conversation</option>
      {#each threads as t (t.id)}<option value={t.id}>{t.title}</option>{/each}
    </select>
    <button class="ghost" title="New conversation" onclick={newThread}>＋</button>
  </div>

  <div class="log" bind:this={scroller}>
    {#if !messages.length && !busy}
      <div class="empty">
        <p class="muted">Ask the coach about the position on the board. Answers are checked against Stockfish.</p>
        {#if hasProvider}
          <div class="sugg">
            {#each suggestions as s}<button class="chip" onclick={() => send(s)}>{s}</button>{/each}
          </div>
        {:else if appCtx?.mode === 'browser'}
          <p class="muted">The coach needs the chessgpt server, which is offline right now. Stockfish still works: turn it on below the board.</p>
        {:else}
          <p><a href="/settings">Add an AI provider</a> (Claude, OpenAI, OpenRouter or a local model) to chat.</p>
        {/if}
      </div>
    {/if}
    {#each messages as m (m.id)}
      <div class="msg {m.role}">
        {#if m.role === 'user'}
          <div class="bubble">{m.text}</div>
        {:else}
          {@render calls(m.tool_calls)}
          <div class="bubble md">{@html renderMarkdown(m.text, m.verification?.unverified_moves ?? [], m.verification?.rejected_moves ?? [])}</div>
          <div class="meta">{@render badge(m.verification)}</div>
        {/if}
      </div>
    {/each}
    {#if busy}
      <div class="msg assistant">
        {@render calls(liveCalls)}
        {#each pendingCalls as c (c.id)}
          <div class="chip pending"><span class="spinner"></span> {TOOL_LABEL[c.name] ?? c.name}…</div>
        {/each}
        {#if streaming}
          <div class="bubble md">{@html renderMarkdown(streaming, liveVerification?.unverified_moves ?? [], liveVerification?.rejected_moves ?? [])}</div>
        {:else if thinking}
          <div class="thinking muted">{thinking}</div>
        {:else if !pendingCalls.length}
          <div class="muted"><span class="spinner"></span> thinking…</div>
        {/if}
        <div class="meta">{@render badge(liveVerification)}</div>
      </div>
    {/if}
    {#if error}<div class="error">{error}</div>{/if}
  </div>

  <form
    class="compose"
    onsubmit={(e) => {
      e.preventDefault();
      send();
    }}
  >
    <textarea
      rows="2"
      placeholder={hasProvider ? 'Ask about this position… (Enter to send)' : 'Configure a provider in Settings first'}
      bind:value={input}
      disabled={!hasProvider}
      onkeydown={(e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
          e.preventDefault();
          send();
        }
      }}
    ></textarea>
    {#if busy}
      <button type="button" onclick={stop}>Stop</button>
    {:else}
      <button class="primary" type="submit" disabled={!input.trim() || !hasProvider}>Ask</button>
    {/if}
  </form>
  <div class="fen muted mono" title="The coach sees this position">{fen}</div>
</div>

<style>
  .chat {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    gap: 0.4rem;
  }
  .bar {
    display: flex;
    gap: 0.3rem;
  }
  .bar select {
    flex: 1;
  }
  .log {
    flex: 1;
    min-height: 8rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.2rem;
  }
  .empty .sugg {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  .sugg .chip {
    cursor: pointer;
  }
  .msg {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .msg.user {
    align-items: flex-end;
  }
  .bubble {
    padding: 0.5rem 0.75rem;
    border-radius: 12px;
    max-width: 95%;
  }
  .user .bubble {
    background: var(--accent);
    color: var(--accent-contrast);
    white-space: pre-wrap;
  }
  .assistant .bubble {
    background: var(--surface-2);
  }
  .md :global(p) {
    margin: 0.25rem 0;
  }
  .md :global(ul) {
    margin: 0.25rem 0;
    padding-left: 1.2rem;
  }
  .md :global(mark.unverified) {
    background: none;
    border-bottom: 2px dotted var(--warn);
    color: inherit;
  }
  .md :global(mark.rejected) {
    background: none;
    color: var(--danger);
    text-decoration: line-through;
  }
  .calls {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .call {
    cursor: pointer;
    font-family: var(--mono);
    font-size: 0.75rem;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .call.err {
    color: var(--danger);
  }
  .pending {
    align-self: flex-start;
  }
  .thinking {
    font-size: 0.8rem;
    font-style: italic;
    white-space: pre-wrap;
    max-height: 4.5rem;
    overflow: hidden;
  }
  .meta {
    font-size: 0.75rem;
  }
  .verified {
    color: var(--ok);
  }
  .partial {
    color: var(--warn);
    cursor: help;
  }
  .compose {
    display: flex;
    gap: 0.4rem;
    align-items: stretch;
  }
  .compose textarea {
    resize: none;
    flex: 1;
  }
  .fen {
    font-size: 0.7rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
