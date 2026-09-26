<script lang="ts">
  import type { DrawShape } from '@lichess-org/chessground/draw';
  import type { Key } from '@lichess-org/chessground/types';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import type { DrillSet } from '$lib/api/types';
  import Board from '$lib/components/Board.svelte';
  import { dests, numberedLine, playMove, playUci, position, turn, uciSquares, whiteWin } from '$lib/chess';
  import { browserEngine } from '$lib/offline/engine';
  import { choose as botChoose, styleFor } from '$lib/training/bot';
  import { KIND_LABEL, accepts, gradeChoice, gradeTap, tally } from '$lib/training/drills';
  import { step } from '$lib/training/solve';

  let set = $state<DrillSet | null>(null);
  let error = $state<string | null>(null);
  let i = $state(0);
  let fen = $state('');
  let at = $state(0);
  let lastMove = $state<[Key, Key] | null>(null);
  let result = $state<'solved' | 'missed' | null>(null);
  let expected = $state<string | null>(null);
  let tapped = $state<string | null>(null);
  let picked = $state<number | null>(null);
  let walkedIntoTrap = $state(false);
  let hints = $state(0);
  let started = 0;
  let again = $state(false);
  // Play-out state.
  let poMoves = $state(0);
  let poWin = $state<number | null>(null);
  let thinking = $state(false);
  let poNote = $state('');
  // Pro mode (set on the drills page): no hints, a visible clock.
  const pro = (() => {
    try {
      return localStorage.getItem('drills.pro') === '1';
    } catch {
      return false;
    }
  })();
  let now = $state(Date.now());
  $effect(() => {
    if (!pro) return;
    const t = setInterval(() => (now = Date.now()), 250);
    return () => clearInterval(t);
  });
  let startedAt = $state(Date.now());
  let endedAt = $state<number | null>(null);
  const clock = $derived(((endedAt ?? now) - startedAt) / 1000);

  const item = $derived(set && i < set.items.length ? set.items[i] : null);
  const orientation = $derived(item ? turn(item.fen) : 'white');
  const moveItem = $derived(item?.kind === 'find' || item?.kind === 'defend' || item?.kind === 'playout');
  const userSide = $derived(item ? turn(item.fen) : 'white');
  const done = $derived(!!set && i >= set.items.length);

  const shapes = $derived.by<DrawShape[]>(() => {
    if (!item) return [];
    const out: DrawShape[] = [];
    const arrow = (uci: string, brush: string) => {
      const sq = uciSquares(uci);
      if (sq) out.push({ orig: sq[0], dest: sq[1], brush });
    };
    const q = item.quiz;
    if (q?.arrow_uci) arrow(q.arrow_uci, 'blue');
    if (q && result && q.answer_squares.length) {
      for (const s of q.answer_squares) out.push({ orig: s as Key, brush: 'green' });
      if (tapped && result === 'missed') out.push({ orig: tapped as Key, brush: 'red' });
    }
    if (result === 'missed' && item.kind === 'find' && expected) arrow(expected, 'green');
    if (result === 'missed' && item.kind === 'defend') item.accept_uci.slice(0, 3).forEach((u) => arrow(u, 'green'));
    if (!result && item.kind === 'find' && hints >= 2) {
      const sq = uciSquares(item.solution_uci[at] ?? '');
      if (sq) out.push({ orig: sq[0], brush: 'yellow' });
    }
    return out;
  });

  onMount(async () => {
    try {
      set = await api.drill(page.params.id!);
      const first = set.items.findIndex((it) => it.solved == null);
      load(first < 0 ? set.items.length : first);
    } catch (e) {
      error = (e as Error).message;
    }
  });

  function load(n: number) {
    i = n;
    at = 0;
    lastMove = null;
    result = null;
    expected = null;
    tapped = null;
    picked = null;
    walkedIntoTrap = false;
    hints = 0;
    poMoves = 0;
    poWin = null;
    poNote = '';
    thinking = false;
    started = Date.now();
    startedAt = started;
    endedAt = null;
    if (set && n < set.items.length) fen = set.items[n].fen;
  }

  async function finish(solved: boolean) {
    if (!set || !item || result) return;
    result = solved ? 'solved' : 'missed';
    endedAt = Date.now();
    try {
      set = await api.drillAttempt(set.id, item.id, { solved, ms: Date.now() - started, hints_used: hints });
    } catch (e) {
      error = (e as Error).message;
    }
  }

  function legalUcis(f: string): string[] {
    const out: string[] = [];
    for (const [o, ds] of dests(f)) for (const d of ds) out.push(o + d);
    return out;
  }

  /** Game over: did the user win (true), draw or lose (false)? null = still on. */
  function outcome(f: string): boolean | null {
    const pos = position(f);
    if (!pos || !pos.isEnd()) return null;
    return pos.outcome()?.winner === userSide;
  }

  /** The user moved in a play-out: check the eval, then the bot replies. */
  async function playoutStep() {
    if (!item?.goal || !set) return;
    const end = outcome(fen);
    if (end !== null) {
      poNote = end ? 'Checkmate.' : 'The game ended without a win.';
      return void finish(end);
    }
    thinking = true;
    try {
      const a = await browserEngine().analyse(fen, { multipv: 4, depth: 12, movetimeMs: 1500 });
      const botWhite = userSide === 'black';
      const cands = a.lines
        .filter((l) => l.pv_uci.length)
        .map((l) => ({ uci: l.pv_uci[0], win: botWhite ? whiteWin(l.score) : 100 - whiteWin(l.score) }));
      poWin = cands.length ? Math.round(100 - Math.max(...cands.map((c) => c.win))) : poWin;
      if (poWin != null && poWin < item.goal.min_win_pct) {
        poNote = `Your winning chances fell to ${poWin}%.`;
        return void finish(false);
      }
      if (poMoves >= item.goal.moves) {
        poNote = `Still winning after ${poMoves} moves (${poWin}%).`;
        return void finish(true);
      }
      const p = playUci(fen, botChoose(cands, legalUcis(fen), styleFor(set.rating)));
      if (!p) return;
      fen = p.fen;
      lastMove = uciSquares(p.uci);
      const after = outcome(fen);
      if (after !== null) {
        poNote = 'The game ended without a win.';
        return void finish(false);
      }
    } catch (e) {
      error = (e as Error).message;
    } finally {
      thinking = false;
    }
  }

  function onmove(orig: Key, dest: Key) {
    if (!item || result || !moveItem || thinking) return;
    const played = playMove(fen, orig, dest);
    if (!played) return;
    if (item.kind === 'playout') {
      fen = played.fen;
      lastMove = [orig, dest];
      poMoves += 1;
      return void playoutStep();
    }
    if (item.kind === 'defend') {
      fen = played.fen;
      lastMove = [orig, dest];
      if (accepts(item.accept_uci, played.uci)) return void finish(true);
      walkedIntoTrap = played.san === item.trap_san[0];
      setTimeout(() => {
        fen = item.fen;
        lastMove = null;
      }, 700);
      return void finish(false);
    }
    const s = step(fen, item.solution_uci, at, played.uci);
    if (s.kind === 'wrong') {
      const before = fen;
      fen = played.fen;
      lastMove = [orig, dest];
      expected = s.expected;
      setTimeout(() => {
        fen = before;
        lastMove = null;
      }, 700);
      return void finish(false);
    }
    fen = s.fen;
    if (s.kind === 'solved') {
      lastMove = [orig, dest];
      return void finish(true);
    }
    lastMove = s.reply ? uciSquares(s.reply) : null;
    at = s.next;
  }

  function onselect(square: Key) {
    const q = item?.quiz;
    if (!q || result || !q.answer_squares.length) return;
    tapped = square;
    void finish(gradeTap(q, square));
  }

  function choose(n: number) {
    const q = item?.quiz;
    if (!q || result) return;
    picked = n;
    void finish(gradeChoice(q, n));
  }

  async function drillAgain() {
    if (!set) return;
    again = true;
    try {
      const next = await api.createDrill({ kind: 'theme', tag: set.technique });
      await goto(`/drills/${next.id}`);
      set = next;
      load(0);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      again = false;
    }
  }
</script>

<svelte:head><title>{set ? `${set.label} drill` : 'Drill'} · chessgpt</title></svelte:head>

{#if error}<p class="error">{error}</p>{/if}

{#if set}
  <div class="head">
    <h1>{set.label}</h1>
    <div class="dots" aria-label="progress">
      {#each set.items as it, n}
        <span
          class="dot"
          class:cur={n === i}
          class:ok={it.solved === true}
          class:bad={it.solved === false}
          title={KIND_LABEL[it.kind]}
        ></span>
      {/each}
    </div>
  </div>

  {#if item}
    <div class="layout">
      <div class="board">
        <Board {fen} {orientation} {lastMove} {shapes} interactive={moveItem && !result && !thinking && turn(fen) === userSide} {onmove} {onselect} />
      </div>
      <aside class="card pad">
        <p class="kind">{KIND_LABEL[item.kind]}{#if item.rating}<span class="muted">&nbsp;· rated {item.rating}</span>{/if}</p>
        <p>{item.prompt}</p>

        {#if item.quiz}
          <p><b>{item.quiz.question}</b></p>
          {#if item.quiz.choices.length}
            <div class="choices">
              {#each item.quiz.choices as c, n}
                <button
                  class:right={result && item.quiz.answer_choice === n}
                  class:wrong={result === 'missed' && picked === n}
                  disabled={!!result}
                  onclick={() => choose(n)}>{c}</button
                >
              {/each}
            </div>
          {:else if !result}
            <p class="muted small">Click a square on the board.</p>
          {/if}
        {/if}

        {#if item.goal}
          <p class="small">
            Move {Math.min(poMoves + (result ? 0 : 1), item.goal.moves)} of {item.goal.moves} · winning chances
            <b>{poWin ?? item.goal.start_win_pct}%</b> (keep above {item.goal.min_win_pct}%){#if thinking} · the bot is thinking…{/if}
          </p>
        {/if}

        {#if pro}<p class="clock mono">{clock.toFixed(1)}s</p>{/if}

        {#if !pro && moveItem && !result && item.hints.length}
          {#each item.hints.slice(0, hints) as h}<p class="hint">💡 {h}</p>{/each}
          {#if hints < item.hints.length}
            <button onclick={() => (hints += 1)}>{hints ? 'Another hint' : 'Hint'}</button>
          {/if}
        {/if}

        {#if poNote}<p class="small">{poNote}</p>{/if}
        {#if result === 'solved'}
          <p class="ok"><b>✓ {item.kind === 'playout' ? 'Converted.' : 'Correct.'}</b></p>
        {:else if result === 'missed'}
          <p class="error">
            <b>✗ Not this time.</b>
            {#if item.kind === 'find'}The green arrow shows the move.{/if}
            {#if item.kind === 'defend'}Green arrows show moves that hold.{/if}
          </p>
          {#if item.kind === 'defend' && item.trap_san.length >= 2}
            <p class="small">
              {walkedIntoTrap ? 'That was the move played in the real game:' : 'The tempting move:'}
              <span class="mono">{item.trap_san[0]}</span> runs into
              <span class="mono">{item.trap_san.slice(1).join(' ')}</span>.
            </p>
          {/if}
          {#if moveItem}<p class="muted small">This position is now in your Train queue.</p>{/if}
        {/if}
        {#if result && item.quiz?.explanation}<p class="small">{item.quiz.explanation}</p>{/if}
        {#if result && item.kind === 'find' && item.line_san.length}
          <p class="small">Line: <span class="mono">{numberedLine(item.fen, item.line_san)}</span></p>
        {/if}
        {#if result}
          <button class="primary" onclick={() => load(i + 1)}>{i + 1 < set.items.length ? 'Next' : 'See results'}</button>
        {/if}
      </aside>
    </div>
  {:else if done}
    {@const rows = tally(set.items)}
    {@const solved = set.items.filter((it) => it.solved).length}
    <section class="card pad summary">
      <h2>{solved} of {set.items.length} solved</h2>
      <table>
        <tbody>
          {#each rows as r}
            <tr><td>{KIND_LABEL[r.kind]}</td><td>{r.solved}/{r.total}</td></tr>
          {/each}
        </tbody>
      </table>
      {#if solved < set.items.length}
        <p class="muted small">The positions you missed are in your <a href="/train">Train</a> queue for spaced review.</p>
      {:else}
        <p class="muted small">Clean sweep. Go again: the next set picks new positions.</p>
      {/if}
      <div class="row">
        <button class="primary" disabled={again} onclick={drillAgain}>{again ? 'Building…' : 'Drill again'}</button>
        <a href="/drills">All drills</a>
      </div>
    </section>
  {/if}
{:else if !error}
  <p class="muted">Loading…</p>
{/if}

<style>
  .pad {
    padding: 1rem 1.2rem;
  }
  .small {
    font-size: 0.85rem;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 1rem;
    justify-content: space-between;
  }
  .dots {
    display: flex;
    gap: 0.3rem;
  }
  .dot {
    width: 0.7rem;
    height: 0.7rem;
    border-radius: 50%;
    border: 1px solid var(--border);
  }
  .dot.cur {
    border-color: currentColor;
  }
  .dot.ok {
    background: var(--ok);
    border-color: var(--ok);
  }
  .dot.bad {
    background: var(--danger);
    border-color: var(--danger);
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
  .kind {
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-size: 0.75rem;
    font-weight: 700;
    margin: 0 0 0.4rem;
  }
  .choices {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.4rem;
  }
  .choices .right {
    border-color: var(--ok);
    color: var(--ok);
    font-weight: 600;
  }
  .choices .wrong {
    border-color: var(--danger);
  }
  .clock {
    font-size: 1.4rem;
    margin: 0.2rem 0 0.6rem;
  }
  .hint {
    font-size: 0.9rem;
    margin: 0.3rem 0;
  }
  .ok {
    color: var(--ok);
  }
  aside button {
    margin-top: 0.5rem;
  }
  .summary table {
    margin: 0.6rem 0;
  }
  .summary td {
    padding: 0.2rem 1.2rem 0.2rem 0;
  }
  .row {
    display: flex;
    gap: 0.6rem;
    align-items: center;
    flex-wrap: wrap;
  }
</style>
