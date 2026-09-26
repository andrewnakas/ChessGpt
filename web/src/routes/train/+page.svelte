<script lang="ts">
  import type { DrawShape } from '@lichess-org/chessground/draw';
  import type { Key } from '@lichess-org/chessground/types';
  import { goto } from '$app/navigation';
  import { getContext, onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import type { Explanation, Meta, Puzzle, PuzzleQueue } from '$lib/api/types';
  import Board from '$lib/components/Board.svelte';
  import { numberedLine, playMove, playUci, tagLabel, turn, uciSquares } from '$lib/chess';
  import { coachAvailable } from '$lib/llm/device.svelte';
  import { LICHESS_THEME, nextLichessPuzzle, type LichessPuzzle } from '$lib/training/lichess';
  import { accepts, drillableTag } from '$lib/training/drills';
  import { step } from '$lib/training/solve';

  const app = getContext<{ meta: Meta | null }>('app');

  type Current =
    | { kind: 'mine'; p: Puzzle }
    | { kind: 'lichess'; p: LichessPuzzle };

  let queue = $state<PuzzleQueue | null>(null);
  let weakTheme = $state<string | null>(null);
  let current = $state<Current | null>(null);
  let fen = $state('');
  let at = $state(0);
  let lastMove = $state<[Key, Key] | null>(null);
  let result = $state<'solved' | 'missed' | null>(null);
  let expected = $state<string | null>(null);
  let explanation = $state<Explanation | null>(null);
  let explaining = $state(false);
  let error = $state<string | null>(null);
  let practised = $state(0);
  let drilling = $state(false);

  /** The technique a drill set would train from this puzzle. */
  const drillTag = $derived(
    current?.kind === 'mine' ? drillableTag(current.p.themes) : current?.kind === 'lichess' ? weakTheme : null
  );

  async function drill() {
    if (!current || !drillTag) return;
    drilling = true;
    try {
      const set = await api.createDrill(
        current.kind === 'mine' ? { kind: 'puzzle', id: current.p.id } : { kind: 'theme', tag: drillTag }
      );
      await goto(`/drills/${set.id}`);
    } catch (e) {
      error = (e as Error).message;
      drilling = false;
    }
  }

  const orientation = $derived(current ? turn(current.p.fen) : 'white');
  const shapes = $derived<DrawShape[]>(
    result === 'missed' && expected ? (([o, d]) => [{ orig: o, dest: d, brush: 'green' }])(uciSquares(expected)!) : []
  );

  onMount(async () => {
    try {
      queue = await api.puzzles();
      const prog = await api.progress().catch(() => null);
      weakTheme = prog?.motifs.map((m) => m.tag).find((t) => LICHESS_THEME[t]) ?? null;
      await next();
    } catch (e) {
      error = (e as Error).message;
    }
  });

  function load(c: Current) {
    current = c;
    fen = c.p.fen;
    at = 0;
    lastMove = null;
    result = null;
    expected = null;
    explanation = null;
  }

  async function next() {
    error = null;
    const mine = queue?.due.shift();
    if (mine) return load({ kind: 'mine', p: mine });
    try {
      load({ kind: 'lichess', p: await nextLichessPuzzle(weakTheme ? LICHESS_THEME[weakTheme] : null) });
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function finish(solved: boolean) {
    result = solved ? 'solved' : 'missed';
    practised += 1;
    if (current?.kind === 'mine') {
      const updated = await api.puzzleAttempt(current.p.id, solved).catch(() => null);
      if (queue && updated && !solved) queue.due.push(updated); // see it again this session
      if (queue && solved) queue.due_count = Math.max(0, queue.due_count - 1);
    }
  }

  function onmove(orig: Key, dest: Key) {
    if (!current || result) return;
    const played = playMove(fen, orig, dest);
    if (!played) return;
    // Defence puzzles: any move that holds counts.
    if (current.kind === 'mine' && current.p.accept_uci.length) {
      fen = played.fen;
      lastMove = [orig, dest];
      if (accepts(current.p.accept_uci, played.uci)) return void finish(true);
      expected = current.p.accept_uci[0];
      const start = current.p.fen;
      setTimeout(() => {
        fen = start;
        lastMove = null;
      }, 600);
      return void finish(false);
    }
    const s = step(fen, current.kind === 'mine' ? current.p.solution_uci : current.p.solution, at, played.uci);
    if (s.kind === 'wrong') {
      fen = played.fen;
      lastMove = [orig, dest];
      expected = s.expected;
      // Put the position back so the arrow shows the better move.
      setTimeout(() => {
        if (current) {
          fen = current.kind === 'mine' ? current.p.fen : fenBefore();
          lastMove = null;
        }
      }, 600);
      void finish(false);
      return;
    }
    if (s.kind === 'solved') {
      fen = s.fen;
      lastMove = [orig, dest];
      void finish(true);
      return;
    }
    fen = s.fen;
    lastMove = s.reply ? uciSquares(s.reply) : null;
    at = s.next;
  }

  /** Position at the current solution step (Lichess puzzles have several). */
  function fenBefore(): string {
    if (!current || current.kind !== 'lichess') return fen;
    let f = current.p.fen;
    for (const u of current.p.solution.slice(0, at)) f = playUci(f, u)?.fen ?? f;
    return f;
  }

  async function explain() {
    if (current?.kind !== 'mine' || !current.p.analysis_id || current.p.ply == null) return;
    explaining = true;
    try {
      explanation = await api.explainPly(current.p.analysis_id, current.p.ply);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      explaining = false;
    }
  }
</script>

<svelte:head><title>Train · chessgpt</title></svelte:head>

<h1>Train</h1>

{#if queue}
  <p class="muted">
    {#if queue.total}
      Your puzzles: <b>{queue.due_count}</b> due · {queue.learned} learned · {queue.total} from your games.
    {:else}
      Puzzles from your own mistakes appear here after you analyse games with your side marked.
    {/if}
    {#if practised}· {practised} done this session{/if}
  </p>
{/if}
{#if error}<p class="error">{error}</p>{/if}

{#if current}
  <div class="layout">
    <div class="board">
      <Board {fen} {orientation} {lastMove} {shapes} interactive={!result} {onmove} />
    </div>
    <aside class="card pad">
      {#if current.kind === 'mine'}
        {#if current.p.source === 'drill'}
          <h2>From a drill</h2>
          <p class="muted small">
            {current.p.accept_uci.length ? 'A careless move here walks into a tactic. Play a safe one' : 'Find the move'}, playing {orientation}.
          </p>
        {:else}
          <h2>From your game</h2>
          <p class="muted small">You went wrong here. Find the move you missed, playing {orientation}.</p>
        {/if}
        {#if current.p.themes.length}
          <p class="small">{#each current.p.themes as t}<span class="chip">{tagLabel(t)}</span> {/each}</p>
        {/if}
      {:else}
        <h2>Practice{#if weakTheme}: {tagLabel(weakTheme)}{/if}</h2>
        <p class="muted small">
          A Lichess puzzle (rated {current.p.rating}){#if weakTheme}, picked because it is your most frequent mistake pattern{/if}.
          Play {orientation}.
        </p>
      {/if}

      {#if result === 'solved'}
        <p class="ok"><b>✓ Correct.</b></p>
        {#if current.kind === 'mine' && current.p.line_san.length}
          <p class="small">The engine's line: <span class="mono">{numberedLine(current.p.fen, current.p.line_san)}</span></p>
        {/if}
      {:else if result === 'missed'}
        <p class="error"><b>✗ Not this time.</b> The green arrow shows the move.</p>
        {#if current.kind === 'mine'}
          {#if current.p.line_san.length}
            <p class="small">The engine's line: <span class="mono">{numberedLine(current.p.fen, current.p.line_san)}</span></p>
          {/if}
          <p class="muted small">It comes back in ten minutes, then less often as you get it right.</p>
          {#if coachAvailable(app.meta?.has_provider) && !explanation}
            <button onclick={explain} disabled={explaining}>{explaining ? 'Asking the coach…' : 'Why?'}</button>
          {/if}
        {/if}
      {/if}
      {#if explanation}
        <div class="expl">
          <b>{explanation.headline}</b>
          <p>{explanation.why_it_matters}</p>
          <p class="muted small">{explanation.takeaway}</p>
        </div>
      {/if}
      {#if result}
        <button class="primary" onclick={next}>Next puzzle</button>
        {#if result === 'missed' && drillTag}
          <button onclick={drill} disabled={drilling}>
            {drilling ? 'Building…' : `Drill ${tagLabel(drillTag).toLowerCase()} ×8`}
          </button>
        {/if}
      {/if}
    </aside>
  </div>
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
  .ok {
    color: var(--ok);
  }
  .expl {
    border-top: 1px solid var(--border);
    margin-top: 0.8rem;
    padding-top: 0.6rem;
  }
  aside button {
    margin-top: 0.6rem;
  }
</style>
