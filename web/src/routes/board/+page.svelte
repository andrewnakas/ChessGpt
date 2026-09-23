<script lang="ts">
  import { page } from '$app/state';
  import type { DrawShape } from '@lichess-org/chessground/draw';
  import type { Key } from '@lichess-org/chessground/types';
  import { getContext } from 'svelte';
  import { streams } from '$lib/api/client';
  import type { EngineAnalysis, Meta } from '$lib/api/types';
  import Board from '$lib/components/Board.svelte';
  import ChatPanel from '$lib/components/ChatPanel.svelte';
  import EngineLines from '$lib/components/EngineLines.svelte';
  import EvalBar from '$lib/components/EvalBar.svelte';
  import { START_FEN, fmtScore, isValidFen, numberedLine, playMove, playUci, uciSquares, whiteWin, type Played } from '$lib/chess';

  const app = getContext<{ meta: Meta | null }>('app');

  const initial = page.url.searchParams.get('fen');
  let startFen = $state(initial && isValidFen(initial) ? initial : START_FEN);
  let line = $state<Played[]>([]);
  let cursor = $state(0);
  let orientation = $state<'white' | 'black'>('white');
  let engineOn = $state(true);
  let live = $state<EngineAnalysis | null>(null);
  let engineError = $state<string | null>(null);
  let fenInput = $state('');
  let fenError = $state<string | null>(null);
  let toolShapes = $state<DrawShape[]>([]);

  const fen = $derived(cursor === 0 ? startFen : line[cursor - 1].fen);
  const lastMove = $derived(cursor > 0 ? uciSquares(line[cursor - 1].uci) : null);
  const movePath = $derived(line.slice(0, cursor).map((m) => m.san));

  $effect(() => {
    fenInput = fen;
  });

  $effect(() => {
    const f = fen;
    if (!engineOn) {
      live = null;
      return;
    }
    const ctrl = new AbortController();
    engineError = null;
    const t = setTimeout(() => {
      streams
        .engine(f, 3, (e) => (e.type === 'error' ? (engineError = e.message) : (live = e.analysis)), ctrl.signal)
        .catch((err) => {
          if ((err as Error).name !== 'AbortError') engineError = (err as Error).message;
        });
    }, 120);
    return () => {
      clearTimeout(t);
      ctrl.abort();
    };
  });

  const shapes: DrawShape[] = $derived.by(() => {
    if (toolShapes.length) return toolShapes;
    if (!live || live.fen !== fen) return [];
    return live.lines.slice(0, 3).flatMap((l, i) => {
      const sq = l.pv_uci[0] ? uciSquares(l.pv_uci[0]) : null;
      return sq ? [{ orig: sq[0], dest: sq[1], brush: i === 0 ? 'paleGreen' : 'paleBlue' }] : [];
    });
  });

  const bar = $derived.by(() => {
    if (live && live.fen === fen && live.lines[0]) return { win: whiteWin(live.lines[0].score), label: fmtScore(live.lines[0].score) };
    return { win: 50, label: '' };
  });

  function onmove(orig: Key, dest: Key) {
    const r = playMove(fen, orig, dest);
    if (!r) return;
    toolShapes = [];
    if (line[cursor]?.uci === r.uci) cursor += 1;
    else {
      line = [...line.slice(0, cursor), r];
      cursor = line.length;
    }
  }

  function playEngineLine(uci: string[]) {
    let f = fen;
    const played: Played[] = [];
    for (const u of uci.slice(0, 12)) {
      const r = playUci(f, u);
      if (!r) break;
      played.push(r);
      f = r.fen;
    }
    line = [...line.slice(0, cursor), ...played];
    cursor = cursor + (played.length ? 1 : 0);
  }

  function setFen(e: Event) {
    e.preventDefault();
    if (!isValidFen(fenInput)) {
      fenError = 'Not a valid FEN.';
      return;
    }
    fenError = null;
    startFen = fenInput.trim();
    line = [];
    cursor = 0;
  }

  function onkey(e: KeyboardEvent) {
    if ((e.target as HTMLElement).closest('input, textarea, select')) return;
    if (e.key === 'ArrowLeft') cursor = Math.max(0, cursor - 1);
    else if (e.key === 'ArrowRight') cursor = Math.min(line.length, cursor + 1);
    else if (e.key === 'Home') cursor = 0;
    else if (e.key === 'End') cursor = line.length;
    else if (e.key === 'f') orientation = orientation === 'white' ? 'black' : 'white';
    else if (e.key === ' ') engineOn = !engineOn;
    else return;
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="layout">
  <section class="left">
    <div class="boardrow">
      <EvalBar win={bar.win} label={bar.label} {orientation} />
      <Board {fen} {orientation} {lastMove} {shapes} {onmove} />
    </div>
    <div class="controls">
      <button onclick={() => (cursor = 0)}>⏮</button>
      <button onclick={() => (cursor = Math.max(0, cursor - 1))}>◀</button>
      <button onclick={() => (cursor = Math.min(line.length, cursor + 1))}>▶</button>
      <button onclick={() => (cursor = line.length)}>⏭</button>
      <button onclick={() => (orientation = orientation === 'white' ? 'black' : 'white')} title="Flip (f)">⇅</button>
      <button
        onclick={() => {
          startFen = START_FEN;
          line = [];
          cursor = 0;
        }}>Reset</button
      >
      {#if line.length}<span class="mono small">{numberedLine(startFen, line.map((m) => m.san))}</span>{/if}
    </div>
    <form class="fen" onsubmit={setFen}>
      <input type="text" class="mono" bind:value={fenInput} aria-label="FEN" />
      <button type="submit">Load FEN</button>
    </form>
    {#if fenError}<p class="error small">{fenError}</p>{/if}
    <div class="card pad">
      <EngineLines analysis={live} on={engineOn} error={engineError} ontoggle={() => (engineOn = !engineOn)} onplay={playEngineLine} />
    </div>
  </section>
  <section class="right card pad chatbox">
    <h2>Ask the coach</h2>
    <ChatPanel
      gameId={null}
      {fen}
      ply={null}
      movePath={startFen === START_FEN ? movePath : []}
      hasProvider={!!app.meta?.has_provider}
      onshapes={(s) => (toolShapes = s)}
    />
  </section>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: minmax(320px, min(calc(100vh - 200px), 660px)) minmax(340px, 1fr);
    gap: 1.2rem;
    align-items: start;
  }
  .left {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .boardrow {
    display: grid;
    grid-template-columns: 22px 1fr;
    gap: 6px;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    align-items: center;
  }
  .fen {
    display: flex;
    gap: 0.4rem;
  }
  .fen input {
    font-size: 0.8rem;
  }
  .small {
    font-size: 0.85rem;
  }
  .pad {
    padding: 0.7rem 0.9rem;
  }
  .chatbox {
    height: calc(100vh - 150px);
    min-height: 30rem;
    display: flex;
    flex-direction: column;
  }
  .chatbox h2 {
    margin: 0 0 0.4rem;
    font-size: 1.05rem;
  }
  @media (max-width: 1000px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }
</style>
