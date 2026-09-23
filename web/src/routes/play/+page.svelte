<script lang="ts">
  import type { Key } from '@lichess-org/chessground/types';
  import { goto } from '$app/navigation';
  import { getContext, onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import type { Meta } from '$lib/api/types';
  import Board from '$lib/components/Board.svelte';
  import { START_FEN, dests, playMove, playUci, position, turn, uciSquares, whiteWin } from '$lib/chess';
  import { coachAvailable } from '$lib/llm/device.svelte';
  import { browserEngine } from '$lib/offline/engine';
  import { choose, styleFor } from '$lib/training/bot';

  const app = getContext<{ meta: Meta | null }>('app');

  let elo = $state(1200);
  let side = $state<'white' | 'black'>('white');
  let playing = $state(false);
  let fen = $state(START_FEN);
  let sans = $state<string[]>([]);
  let lastMove = $state<[Key, Key] | null>(null);
  let thinking = $state(false);
  let result = $state<string | null>(null);
  let resultText = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);
  let stop: AbortController | null = null;

  onMount(async () => {
    const p = await api.progress().catch(() => null);
    const s = await api.settings().catch(() => null);
    elo = p?.rolling_estimate ?? s?.elo ?? 1200;
  });

  function legalUcis(f: string): string[] {
    const out: string[] = [];
    for (const [o, ds] of dests(f)) for (const d of ds) out.push(o + d);
    return out;
  }

  function checkEnd(): boolean {
    const pos = position(fen);
    if (!pos || !pos.isEnd()) return false;
    const o = pos.outcome();
    result = !o?.winner ? '1/2-1/2' : o.winner === 'white' ? '1-0' : '0-1';
    resultText = !o?.winner ? 'Draw.' : o.winner === side ? 'You won.' : 'The bot won.';
    playing = false;
    return true;
  }

  async function botMove() {
    if (!playing || turn(fen) === side) return;
    thinking = true;
    stop = new AbortController();
    try {
      const style = styleFor(elo);
      const a = await browserEngine().analyse(fen, { multipv: 4, depth: style.depth, movetimeMs: 3000, signal: stop.signal });
      const botWhite = side === 'black';
      const cands = a.lines
        .filter((l) => l.pv_uci.length)
        .map((l) => ({ uci: l.pv_uci[0], win: botWhite ? whiteWin(l.score) : 100 - whiteWin(l.score) }));
      const uci = choose(cands, legalUcis(fen), style);
      const p = playUci(fen, uci);
      if (!p || !playing) return;
      fen = p.fen;
      sans = [...sans, p.san];
      lastMove = uciSquares(p.uci);
      checkEnd();
    } catch (e) {
      if (playing) error = (e as Error).message;
    } finally {
      thinking = false;
    }
  }

  function start() {
    error = null;
    fen = START_FEN;
    sans = [];
    lastMove = null;
    result = null;
    playing = true;
    if (side === 'black') void botMove();
  }

  function resign() {
    stop?.abort();
    playing = false;
    result = side === 'white' ? '0-1' : '1-0';
    resultText = 'You resigned.';
  }

  function onmove(orig: Key, dest: Key) {
    if (!playing || thinking || turn(fen) !== side) return;
    const p = playMove(fen, orig, dest);
    if (!p) return;
    fen = p.fen;
    sans = [...sans, p.san];
    lastMove = [orig, dest];
    if (!checkEnd()) void botMove();
  }

  function pgn(): string {
    const bot = `chessgpt bot (${elo})`;
    const tags = [
      ['Event', 'Sparring game'],
      ['Date', new Date().toISOString().slice(0, 10).replaceAll('-', '.')],
      ['White', side === 'white' ? 'You' : bot],
      ['Black', side === 'black' ? 'You' : bot],
      ['Result', result ?? '*'],
      [side === 'white' ? 'BlackElo' : 'WhiteElo', String(elo)]
    ];
    const moves = sans.map((s, i) => (i % 2 === 0 ? `${i / 2 + 1}. ${s}` : s)).join(' ');
    return tags.map(([k, v]) => `[${k} "${v}"]`).join('\n') + `\n\n${moves} ${result ?? '*'}\n`;
  }

  async function analyse() {
    busy = true;
    error = null;
    try {
      const r = await api.importGames({ source: 'pgn', pgn: pgn() });
      const g = r.games[0];
      if (!g) throw new Error(r.errors[0] ?? 'could not save the game');
      await api.setSide(g.id, side);
      await api.analyse(g.id, { elo, user_side: side, explain: coachAvailable(app.meta?.has_provider), force: false });
      await goto(`/analyse/${g.id}`);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Play · chessgpt</title></svelte:head>

<h1>Play</h1>

<div class="layout">
  <div class="board">
    <Board {fen} orientation={side} {lastMove} interactive={playing && !thinking && turn(fen) === side} {onmove} />
  </div>
  <aside class="card pad">
    {#if !playing && !result}
      <h2>Spar with a bot at your level</h2>
      <p class="muted small">
        The bot is Stockfish held back to play like a person of the rating you pick: shallower search and the odd mistake.
        Afterwards the game is analysed like any other, and your mistakes become puzzles.
      </p>
      <label>Bot rating <b>{elo}</b>
        <input type="range" min="600" max="2600" step="50" bind:value={elo} />
      </label>
      <div class="sides">
        <label><input type="radio" bind:group={side} value="white" /> Play white</label>
        <label><input type="radio" bind:group={side} value="black" /> Play black</label>
      </div>
      <button class="primary" onclick={start}>Start</button>
    {:else if playing}
      <h2>vs bot ({elo})</h2>
      <p class="muted">{thinking ? 'The bot is thinking…' : turn(fen) === side ? 'Your move.' : ''}</p>
      <p class="small mono">{sans.length} moves</p>
      <button class="ghost danger" onclick={resign}>Resign</button>
    {:else}
      <h2>{resultText}</h2>
      <p class="muted small">{sans.length} moves.</p>
      <button class="primary" onclick={analyse} disabled={busy || sans.length < 2}>{busy ? 'Saving…' : 'Analyse this game'}</button>
      <button onclick={() => (result = null)}>New game</button>
    {/if}
    {#if error}<p class="error small">{error}</p>{/if}
  </aside>
</div>

<style>
  .pad {
    padding: 1rem 1.2rem;
  }
  .small {
    font-size: 0.85rem;
  }
  h2 {
    font-size: 1.05rem;
    margin: 0 0 0.5rem;
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(0, 560px) minmax(16rem, 1fr);
    gap: 1.2rem;
    align-items: start;
  }
  @media (max-width: 800px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }
  label {
    display: block;
    margin: 0.6rem 0;
  }
  input[type='range'] {
    width: 100%;
  }
  .sides {
    display: flex;
    gap: 1rem;
  }
  .sides label {
    display: inline-flex;
    gap: 0.3rem;
    align-items: center;
  }
  aside button {
    margin-top: 0.6rem;
  }
</style>
