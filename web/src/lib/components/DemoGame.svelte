<script lang="ts">
  // The home-page demo: a famous game, reviewed the way chessgpt reviews
  // yours, with "find the move" challenges at the turning points.
  import type { DrawShape } from '@lichess-org/chessground/draw';
  import type { Key } from '@lichess-org/chessground/types';
  import type { Classification, Score } from '$lib/api/types';
  import Board from '$lib/components/Board.svelte';
  import { CLASS_LABEL, fmtScore, isError, numberedLine, playMove, uciSquares, type PromotionRole } from '$lib/chess';
  import data from '$lib/demo/opera.json';
  import { NOTES } from '$lib/demo/notes';

  interface DemoMove {
    ply: number;
    mover: 'white' | 'black';
    san: string;
    uci: string;
    fen_before: string;
    fen_after: string;
    score: Score;
    classification: Classification;
    best_uci: string | null;
    best_line_san: string[];
    win_white: number;
  }
  const moves = data.moves as DemoMove[];

  /** Plies shown so far (0 = start position). */
  let ply = $state(0);
  /** Asking the viewer to find the move at `quiz` (a ply). */
  let quiz = $state<number | null>(null);
  let quizResult = $state<'right' | 'wrong' | null>(null);
  let tryFen = $state<string | null>(null);
  const solved = $state<Record<number, boolean>>({});

  const cur = $derived(ply > 0 ? moves[ply - 1] : null);
  const fen = $derived(tryFen ?? (quiz ? moves[quiz - 1].fen_before : cur ? cur.fen_after : data.start_fen));
  const note = $derived(cur ? NOTES[cur.ply]?.text : null);
  const lastMove = $derived<[Key, Key] | null>(quiz || !cur ? null : uciSquares(cur.uci));
  const shapes = $derived.by<DrawShape[]>(() => {
    if (quiz || !cur || !isError(cur.classification) || !cur.best_uci) return [];
    const sq = uciSquares(cur.best_uci);
    return sq ? [{ orig: sq[0], dest: sq[1], brush: 'green' }] : [];
  });

  function go(n: number) {
    quiz = null;
    quizResult = null;
    tryFen = null;
    ply = Math.max(0, Math.min(moves.length, n));
  }

  function next() {
    if (quiz) return go(quiz);
    const upcoming = ply + 1;
    if (upcoming <= moves.length && NOTES[upcoming]?.quiz && solved[upcoming] === undefined) {
      quiz = upcoming;
      quizResult = null;
      tryFen = null;
      return;
    }
    go(upcoming);
  }

  function onmove(orig: Key, dest: Key, promotion?: PromotionRole) {
    if (!quiz || quizResult) return;
    const m = moves[quiz - 1];
    const played = playMove(m.fen_before, orig, dest, promotion);
    if (!played) return;
    const right = played.uci.slice(0, 4) === m.uci.slice(0, 4);
    solved[quiz] = right;
    quizResult = right ? 'right' : 'wrong';
    tryFen = played.fen;
    const q = quiz;
    setTimeout(() => {
      if (quiz === q) go(q);
    }, right ? 900 : 1600);
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === 'ArrowRight') next();
    if (e.key === 'ArrowLeft') go(ply - 1);
  }

  const points = $derived(
    [50, ...moves.map((m) => m.win_white)].map((w, i) => `${(i / moves.length) * 100},${100 - w}`).join(' ')
  );
  const glyph = (m: DemoMove) =>
    m.ply === 19 || m.ply === 31 ? '!!' : m.classification === 'mistake' ? '?' : m.classification === 'blunder' ? '??' : m.classification === 'inaccuracy' ? '?!' : '';
</script>

<svelte:window onkeydown={onkey} />

<section class="demo card">
  <header>
    <div>
      <h2>See it on a famous game</h2>
      <p class="muted small">Paul Morphy vs Duke Karl &amp; Count Isouard, Paris 1858: the Opera Game. Step through it, or try to find Morphy’s moves.</p>
    </div>
    <span class="muted small acc">Accuracy: White {data.accuracy.white?.toFixed(0)}% · Black {data.accuracy.black?.toFixed(0)}%</span>
  </header>
  <div class="grid">
    <div class="boardcol">
      <Board {fen} orientation="white" {lastMove} {shapes} interactive={!!quiz && !quizResult} {onmove} />
      <div class="controls">
        <button onclick={() => go(0)} aria-label="Start">⏮</button>
        <button onclick={() => go(ply - 1)} aria-label="Back">◀</button>
        <button class="primary" onclick={next}>{quiz ? 'Show me' : ply === 0 ? 'Start ▶' : 'Next ▶'}</button>
        <button onclick={() => go(moves.length)} aria-label="End">⏭</button>
      </div>
    </div>
    <div class="side">
      <svg class="graph" viewBox="0 0 100 100" preserveAspectRatio="none" role="img" aria-label="White's winning chances over the game">
        <line x1="0" y1="50" x2="100" y2="50" class="mid" />
        <polyline points={points} />
        <line x1={(ply / moves.length) * 100} x2={(ply / moves.length) * 100} y1="0" y2="100" class="now" />
      </svg>
      <div class="panel">
        {#if quiz}
          <p class="kind">Your move</p>
          <p>{NOTES[quiz].quiz}</p>
          {#if quizResult === 'right'}<p class="ok"><b>✓ That’s Morphy’s move.</b></p>{/if}
          {#if quizResult === 'wrong'}<p class="bad"><b>Not quite.</b> Here is what Morphy played…</p>{/if}
          {#if !quizResult}<p class="muted small">Drag a piece on the board, or press “Show me”.</p>{/if}
        {:else if cur}
          <p class="move">
            <span class="c-{cur.classification}">{numberedLine(cur.fen_before, [cur.san])}{glyph(cur)}</span>
            <span class="chip">{CLASS_LABEL[cur.classification]}</span>
            <span class="muted small">eval {fmtScore(cur.score)}</span>
          </p>
          {#if solved[cur.ply] !== undefined}
            <p class="small {solved[cur.ply] ? 'ok' : 'muted'}">{solved[cur.ply] ? 'You found it.' : 'One to remember.'}</p>
          {/if}
          {#if note}<p class="note">{note}</p>{/if}
          {#if isError(cur.classification) && cur.best_line_san.length}
            <p class="small">Better: <span class="mono">{numberedLine(cur.fen_before, cur.best_line_san.slice(0, 4))}</span> (green arrow)</p>
          {/if}
        {:else}
          <p>chessgpt runs Stockfish over every move, marks the mistakes, and explains the moments that decided the game.</p>
          <p class="muted small">Press Start, or use the ← → keys.</p>
        {/if}
      </div>
      <ol class="moves">
        {#each moves as m (m.ply)}
          {#if m.mover === 'white'}<li class="num">{(m.ply + 1) / 2}.</li>{/if}
          <li>
            <button class="mv c-{isError(m.classification) ? m.classification : 'plain'}" class:on={m.ply === ply && !quiz} onclick={() => go(m.ply)}>
              {m.san}{glyph(m)}
            </button>
          </li>
        {/each}
      </ol>
    </div>
  </div>
</section>

<style>
  .demo {
    padding: 1rem 1.2rem;
    margin-top: 1.5rem;
    max-width: 62rem;
  }
  header {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
  }
  h2 {
    font-size: 1.2rem;
    margin: 0 0 0.2rem;
  }
  .small {
    font-size: 0.85rem;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 26rem) minmax(0, 1fr);
    gap: 1.2rem;
    margin-top: 0.8rem;
  }
  @media (max-width: 760px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
  .controls {
    display: flex;
    gap: 0.4rem;
    justify-content: center;
    margin-top: 0.5rem;
  }
  .graph {
    width: 100%;
    height: 4.5rem;
    background: var(--surface-2);
    border-radius: 6px;
  }
  .graph polyline {
    fill: none;
    stroke: var(--text);
    stroke-width: 1.2;
    vector-effect: non-scaling-stroke;
  }
  .graph .mid {
    stroke: var(--border);
    vector-effect: non-scaling-stroke;
  }
  .graph .now {
    stroke: var(--accent);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
  .panel {
    min-height: 9rem;
    margin: 0.6rem 0;
  }
  .kind {
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-size: 0.75rem;
    font-weight: 700;
    margin: 0 0 0.3rem;
    color: var(--accent);
  }
  .move {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: baseline;
    font-weight: 600;
  }
  .note {
    line-height: 1.5;
  }
  .ok {
    color: var(--ok);
  }
  .bad {
    color: var(--danger);
  }
  .moves {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.1rem 0.2rem;
    font-size: 0.9rem;
  }
  .num {
    color: var(--muted);
    padding: 0.15rem 0 0.15rem 0.3rem;
  }
  .mv {
    border: 0;
    background: none;
    padding: 0.15rem 0.3rem;
    border-radius: 5px;
    font: inherit;
    cursor: pointer;
    color: inherit;
  }
  .mv.on {
    background: var(--surface-2);
    font-weight: 700;
  }
  .mv.c-mistake {
    color: var(--c-mistake);
  }
  .mv.c-inaccuracy {
    color: var(--c-inaccuracy);
  }
  .mv.c-blunder {
    color: var(--c-blunder);
  }
</style>
