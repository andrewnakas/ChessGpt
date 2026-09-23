<script lang="ts">
  import { page } from '$app/state';
  import type { DrawShape } from '@lichess-org/chessground/draw';
  import type { Key } from '@lichess-org/chessground/types';
  import { getContext, onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { api, streams } from '$lib/api/client';
  import type {
    EngineAnalysis,
    Explanation,
    GameAnalysis,
    GameDetail,
    JobEvent,
    JobStage,
    Meta,
    MoveEval,
    Side
  } from '$lib/api/types';
  import Board from '$lib/components/Board.svelte';
  import ChatPanel from '$lib/components/ChatPanel.svelte';
  import EngineLines from '$lib/components/EngineLines.svelte';
  import EvalBar from '$lib/components/EvalBar.svelte';
  import EvalGraph, { type GraphPoint } from '$lib/components/EvalGraph.svelte';
  import ExplanationPanel from '$lib/components/ExplanationPanel.svelte';
  import MoveList from '$lib/components/MoveList.svelte';
  import {
    fmtScore,
    isError,
    numberedLine,
    playMove,
    playSan,
    playUci,
    tagLabel,
    uciSquares,
    whiteWin,
    type Played
  } from '$lib/chess';

  const app = getContext<{ meta: Meta | null }>('app');
  const id = $derived(page.params.id as string);

  let game = $state<GameDetail | null>(null);
  let analysis = $state<GameAnalysis | null>(null);
  let loadError = $state<string | null>(null);
  let ply = $state(0);
  let variation = $state<Played[]>([]);
  let future = $state<Played[]>([]);
  let orientation = $state<'white' | 'black'>('white');
  let engineOn = $state(false);
  let live = $state<EngineAnalysis | null>(null);
  let engineError = $state<string | null>(null);
  const pending = new SvelteSet<number>();
  let failed = $state<Record<number, string>>({});
  let progress = $state<{ stage: JobStage; done: number; total: number } | null>(null);
  let jobError = $state<string | null>(null);
  let toolShapes = $state<DrawShape[]>([]);
  let jobCtrl: AbortController | null = null;

  // analyse form
  let elo = $state(1500);
  let side = $state<Side | 'none'>('none');
  let explain = $state(true);
  let starting = $state(false);

  onMount(() => {
    load();
    return () => jobCtrl?.abort();
  });

  async function load() {
    try {
      const [g, s] = await Promise.all([api.game(id), api.settings()]);
      game = g;
      elo = s.elo;
      side = g.summary.user_side ?? 'none';
      orientation = g.summary.user_side ?? 'white';
      explain = app.meta?.has_provider ?? false;
      analysis = g.analysis;
      if (analysis && (analysis.status === 'queued' || analysis.status === 'running')) listen(analysis.id);
      const firstKey = analysis?.key_moments[0];
      const linked = parseInt(page.url.searchParams.get('ply') ?? '', 10);
      if (linked > 0 && linked <= g.moves.length) ply = linked;
      else if (analysis?.status === 'done' && firstKey) ply = firstKey;
    } catch (e) {
      loadError = (e as Error).message;
    }
  }

  function listen(aid: string) {
    jobCtrl?.abort();
    jobCtrl = new AbortController();
    jobError = null;
    streams.job(aid, onJob, jobCtrl.signal).catch((e) => {
      if ((e as Error).name !== 'AbortError') jobError = (e as Error).message;
    });
  }

  function upsertMove(m: MoveEval) {
    if (!analysis) return;
    const moves = analysis.moves.filter((x) => x.ply !== m.ply);
    moves.push(m);
    moves.sort((a, b) => a.ply - b.ply);
    analysis = { ...analysis, moves };
  }

  function upsertExplanation(e: Explanation) {
    if (!analysis) return;
    analysis = { ...analysis, explanations: [...analysis.explanations.filter((x) => x.ply !== e.ply), e] };
  }

  function onJob(e: JobEvent) {
    switch (e.type) {
      case 'snapshot':
        analysis = e.analysis;
        break;
      case 'move':
        upsertMove(e.eval);
        break;
      case 'progress':
        progress = { stage: e.stage, done: e.done, total: e.total };
        break;
      case 'accuracy':
        if (analysis) analysis = { ...analysis, white_accuracy: e.white, black_accuracy: e.black };
        break;
      case 'key_moments':
        if (analysis) {
          const keys = new Set(e.plies);
          analysis = {
            ...analysis,
            key_moments: e.plies,
            moves: analysis.moves.map((m) => ({ ...m, is_key_moment: keys.has(m.ply) }))
          };
        }
        break;
      case 'explanation_started':
        pending.add(e.ply);
        break;
      case 'explanation':
        pending.delete(e.explanation.ply);
        upsertExplanation(e.explanation);
        break;
      case 'explanation_failed':
        pending.delete(e.ply);
        failed = { ...failed, [e.ply]: e.message };
        break;
      case 'review':
        if (analysis) analysis = { ...analysis, review: e.review };
        break;
      case 'done':
        analysis = e.analysis;
        progress = null;
        pending.clear();
        if (game) game = { ...game, summary: { ...game.summary, analysis_status: e.analysis.status } };
        break;
      case 'error':
        jobError = e.message;
        break;
    }
  }

  async function startAnalysis(force = false) {
    if (!game) return;
    starting = true;
    jobError = null;
    try {
      const r = await api.analyse(game.summary.id, {
        elo,
        user_side: side === 'none' ? null : side,
        explain,
        force
      });
      if (side !== 'none') orientation = side;
      analysis = null;
      listen(r.analysis_id);
    } catch (e) {
      jobError = (e as Error).message;
    } finally {
      starting = false;
    }
  }

  // ------------------------------------------------------------ position

  const N = $derived(game?.moves.length ?? 0);
  const evals = $derived(new Map((analysis?.moves ?? []).map((m) => [m.ply, m])));
  const explanations = $derived(new Map((analysis?.explanations ?? []).map((e) => [e.ply, e])));
  const mainFen = (p: number) => (!game ? '' : p === 0 ? game.start_fen : game.moves[p - 1].fen_after);
  const currentFen = $derived(variation.length ? variation[variation.length - 1].fen : mainFen(ply));
  const lastMove = $derived.by(() => {
    if (variation.length) return uciSquares(variation[variation.length - 1].uci);
    if (game && ply > 0) return uciSquares(game.moves[ply - 1].uci);
    return null;
  });
  const movePath = $derived([...(game?.moves.slice(0, ply).map((m) => m.san) ?? []), ...variation.map((v) => v.san)]);
  const currentMove = $derived(ply > 0 && !variation.length ? evals.get(ply) : undefined);
  const exploring = $derived(variation.length > 0 || future.length > 0);

  const shapes: DrawShape[] = $derived.by(() => {
    if (toolShapes.length) return toolShapes;
    if (engineOn && live && live.fen === currentFen) {
      return live.lines.slice(0, 3).flatMap((l, i) => {
        const sq = l.pv_uci[0] ? uciSquares(l.pv_uci[0]) : null;
        return sq ? [{ orig: sq[0], dest: sq[1], brush: i === 0 ? 'paleGreen' : 'paleBlue' }] : [];
      });
    }
    if (currentMove && isError(currentMove.classification) && currentMove.best_uci) {
      const sq = uciSquares(currentMove.best_uci);
      if (sq) return [{ orig: sq[0], dest: sq[1], brush: 'green' }];
    }
    return [];
  });

  const bar = $derived.by(() => {
    if (engineOn && live && live.fen === currentFen) {
      if (live.terminal === 'checkmate') {
        const whiteToMove = currentFen.split(' ')[1] === 'w';
        return { win: whiteToMove ? 0 : 100, label: '#' };
      }
      if (live.lines[0]) return { win: whiteWin(live.lines[0].score), label: fmtScore(live.lines[0].score) };
    }
    if (!variation.length) {
      if (currentMove) return { win: whiteWin(currentMove.score), label: fmtScore(currentMove.score) };
      if (ply === 0 && analysis?.start_score) return { win: whiteWin(analysis.start_score), label: fmtScore(analysis.start_score) };
    }
    return { win: 50, label: '' };
  });

  const graph: GraphPoint[] = $derived.by(() => {
    if (!analysis?.moves.length) return [];
    const start = analysis.start_score ? whiteWin(analysis.start_score) : 50;
    return [
      { ply: 0, win: start },
      ...analysis.moves.map((m) => ({
        ply: m.ply,
        win: m.mover === 'white' ? m.win_after : 100 - m.win_after,
        cls: m.classification,
        key: m.is_key_moment,
        san: m.san
      }))
    ];
  });

  // Live engine: restart whenever the position changes.
  $effect(() => {
    const fen = currentFen;
    if (!engineOn || !fen) {
      live = null;
      return;
    }
    const ctrl = new AbortController();
    engineError = null;
    const t = setTimeout(() => {
      streams
        .engine(
          fen,
          3,
          (e) => {
            if (e.type === 'error') engineError = e.message;
            else live = e.analysis;
          },
          ctrl.signal
        )
        .catch((err) => {
          if ((err as Error).name !== 'AbortError') engineError = (err as Error).message;
        });
    }, 120);
    return () => {
      clearTimeout(t);
      ctrl.abort();
    };
  });

  // ------------------------------------------------------------ navigation

  function select(p: number) {
    ply = Math.max(0, Math.min(N, p));
    variation = [];
    future = [];
    toolShapes = [];
  }

  function next() {
    if (future.length) {
      variation = [...variation, future[0]];
      future = future.slice(1);
    } else if (!variation.length) select(ply + 1);
  }

  function prev() {
    if (variation.length) {
      future = [variation[variation.length - 1], ...future];
      variation = variation.slice(0, -1);
    } else select(ply - 1);
  }

  function jumpKey(dir: 1 | -1) {
    const keys = analysis?.key_moments ?? [];
    const target = dir > 0 ? keys.find((k) => k > ply) : [...keys].reverse().find((k) => k < ply);
    if (target !== undefined) select(target);
  }

  function onmove(orig: Key, dest: Key) {
    const r = playMove(currentFen, orig, dest);
    if (!r || !game) return;
    toolShapes = [];
    if (!variation.length && game.moves[ply]?.uci === r.uci) {
      select(ply + 1);
      return;
    }
    variation = [...variation, r];
    future = future[0]?.uci === r.uci ? future.slice(1) : [];
  }

  /** Put a SAN line (from the position before the current move) on the board. */
  function showLine(sans: string[]) {
    if (!currentMove && !variation.length) return;
    const base = currentMove ? ply - 1 : ply;
    let fen = mainFen(base);
    const played: Played[] = [];
    for (const s of sans) {
      const r = playSan(fen, s);
      if (!r) break;
      played.push(r);
      fen = r.fen;
    }
    if (!played.length) return;
    ply = base;
    variation = [played[0]];
    future = played.slice(1);
    toolShapes = [];
  }

  function playEngineLine(uci: string[]) {
    let fen = currentFen;
    const played: Played[] = [];
    for (const u of uci.slice(0, 12)) {
      const r = playUci(fen, u);
      if (!r) break;
      played.push(r);
      fen = r.fen;
    }
    if (!played.length) return;
    variation = [...variation, played[0]];
    future = played.slice(1);
  }

  let shareMsg = $state<string | null>(null);
  async function shareGame() {
    if (!game) return;
    const r = await api.share(game.summary.id);
    const url = `${location.origin}${r.path}`;
    try {
      await navigator.clipboard.writeText(url);
      shareMsg = 'Link copied';
    } catch {
      shareMsg = url;
    }
    setTimeout(() => (shareMsg = null), 3000);
  }

  async function explainNow() {
    if (!analysis || !currentMove) return;
    const p = currentMove.ply;
    pending.add(p);
    const { [p]: _, ...rest } = failed;
    failed = rest;
    try {
      upsertExplanation(await api.explainPly(analysis.id, p));
    } catch (e) {
      failed = { ...failed, [p]: (e as Error).message };
    } finally {
      pending.delete(p);
    }
  }

  function onkey(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    if (t.closest('input, textarea, select, [contenteditable]')) return;
    const handled: Record<string, () => void> = {
      ArrowLeft: prev,
      ArrowRight: next,
      ArrowUp: () => jumpKey(-1),
      ArrowDown: () => jumpKey(1),
      Home: () => select(0),
      End: () => select(N),
      f: () => (orientation = orientation === 'white' ? 'black' : 'white'),
      ' ': () => (engineOn = !engineOn),
      Escape: () => select(ply)
    };
    const f = handled[e.key];
    if (f) {
      e.preventDefault();
      f();
    }
  }

  const players = $derived.by(() => {
    if (!game) return null;
    const s = game.summary;
    const white = { name: s.white, elo: s.white_elo, acc: analysis?.white_accuracy ?? null, side: 'white' as const };
    const black = { name: s.black, elo: s.black_elo, acc: analysis?.black_accuracy ?? null, side: 'black' as const };
    return orientation === 'white' ? { top: black, bottom: white } : { top: white, bottom: black };
  });

  const counts = $derived.by(() => {
    const c = { white: { inaccuracy: 0, mistake: 0, blunder: 0 }, black: { inaccuracy: 0, mistake: 0, blunder: 0 } };
    for (const m of analysis?.moves ?? []) {
      const k = m.classification === 'missed_win' ? 'blunder' : m.classification;
      if (k === 'inaccuracy' || k === 'mistake' || k === 'blunder') c[m.mover][k] += 1;
    }
    return c;
  });

  const stageLabel: Record<JobStage, string> = {
    engine: 'Stockfish is analysing every move',
    deep_pass: 'Looking deeper at the key moments',
    explaining: 'The coach is writing explanations',
    review: 'Writing the game review'
  };
</script>

<svelte:window onkeydown={onkey} />

{#snippet player(p: { name: string; elo: number | null; acc: number | null; side: 'white' | 'black' })}
  <div class="player">
    <span class="dot {p.side}"></span>
    <b>{p.name}</b>
    {#if p.elo}<span class="muted">{p.elo}</span>{/if}
    {#if game?.summary.user_side === p.side}<span class="chip">you</span>{/if}
    <span class="spacer"></span>
    {#if p.acc !== null}
      <span class="acc" title="Lichess-style accuracy">{p.acc.toFixed(1)}%</span>
      <span class="counts mono">
        <span class="c-inaccuracy">{counts[p.side].inaccuracy}?!</span>
        <span class="c-mistake">{counts[p.side].mistake}?</span>
        <span class="c-blunder">{counts[p.side].blunder}??</span>
      </span>
    {/if}
  </div>
{/snippet}

{#if loadError}
  <p class="error">{loadError}</p>
{:else if !game || !players}
  <p class="muted"><span class="spinner"></span> Loading game…</p>
{:else}
  <div class="layout">
    <section class="left">
      {@render player(players.top)}
      <div class="boardrow">
        <div class="evalbar"><EvalBar win={bar.win} label={bar.label} {orientation} /></div>
        <div class="boardbox"><Board fen={currentFen} {orientation} {lastMove} {shapes} {onmove} /></div>
      </div>
      {@render player(players.bottom)}
      <div class="controls">
        <button onclick={() => select(0)} title="Start (Home)">⏮</button>
        <button onclick={prev} title="Back (←)">◀</button>
        <button onclick={next} title="Forward (→)">▶</button>
        <button onclick={() => select(N)} title="End (End)">⏭</button>
        <button onclick={() => jumpKey(-1)} title="Previous key moment (↑)" disabled={!analysis?.key_moments.length}>◆◀</button>
        <button onclick={() => jumpKey(1)} title="Next key moment (↓)" disabled={!analysis?.key_moments.length}>▶◆</button>
        <button onclick={() => (orientation = orientation === 'white' ? 'black' : 'white')} title="Flip (f)">⇅</button>
        {#if exploring}
          <span class="explore">
            Exploring: <span class="mono">{numberedLine(mainFen(ply), variation.map((v) => v.san))}</span>
            {#if future.length}<span class="muted mono"> {future.map((f) => f.san).join(' ')}</span>{/if}
            <button class="ghost" onclick={() => select(ply)}>Back to game</button>
          </span>
        {/if}
      </div>
      <div class="card pad">
        <EngineLines analysis={live} on={engineOn} error={engineError} ontoggle={() => (engineOn = !engineOn)} onplay={playEngineLine} />
      </div>
      {#if graph.length}
        <EvalGraph points={graph} current={variation.length ? -1 : ply} onselect={select} />
      {/if}
    </section>

    <section class="right">
      <div class="card pad info">
        <div class="title">
          <b>{game.summary.white}</b> vs <b>{game.summary.black}</b>
          <span class="mono res">{game.summary.result}</span>
        </div>
        <div class="muted small">
          {#if game.summary.eco}<span class="mono">{game.summary.eco}</span>{/if}
          {game.summary.opening ?? ''}
          {#if game.summary.date}· {game.summary.date}{/if}
          {#if game.summary.time_control}· {game.summary.time_control}{/if}
        </div>

        {#if analysis && (analysis.status === 'running' || analysis.status === 'queued' || progress)}
          <div class="progress">
            <span class="spinner"></span>
            {progress ? stageLabel[progress.stage] : 'Starting analysis'}
            {#if progress && progress.total}<span class="muted">{progress.done}/{progress.total}</span>{/if}
            <button class="ghost small" onclick={() => analysis && api.cancelAnalysis(analysis.id)}>Cancel</button>
          </div>
          {#if progress?.stage === 'engine' && progress.total}
            <div class="meter"><div style="width:{(100 * progress.done) / progress.total}%"></div></div>
          {/if}
        {:else if !analysis || analysis.status === 'failed' || analysis.status === 'cancelled'}
          <form
            class="analyse"
            onsubmit={(e) => {
              e.preventDefault();
              startAnalysis(!!analysis);
            }}
          >
            <label>Your rating <input type="number" min="100" max="3500" step="50" bind:value={elo} /></label>
            <label>
              You played
              <select bind:value={side}>
                <option value="none">Not me / both</option>
                <option value="white">White</option>
                <option value="black">Black</option>
              </select>
            </label>
            <label class="check" title={app.meta?.has_provider ? '' : 'Add an AI provider in Settings'}>
              <input type="checkbox" bind:checked={explain} disabled={!app.meta?.has_provider} /> Coach explanations
            </label>
            <button class="primary" type="submit" disabled={starting}>
              {analysis ? 'Analyse again' : 'Analyse game'}
            </button>
          </form>
          {#if analysis?.error}<p class="error small">{analysis.error}</p>{/if}
        {:else if analysis.status === 'done'}
          <div class="done muted small">
            Analysed for {analysis.elo} ({analysis.tier}) with {analysis.engine}.
            <button class="ghost small" onclick={() => startAnalysis(true)}>Re-run</button>
            <button class="ghost small" onclick={shareGame}>{shareMsg ?? 'Share link'}</button>
          </div>
        {/if}
        {#if jobError}<p class="error small">{jobError}</p>{/if}

        {#if analysis?.review}
          <div class="review">
            <p>{analysis.review.text}</p>
            <div>
              {#each analysis.review.themes as t}<span class="chip">Work on: {tagLabel(t)}</span>{/each}
            </div>
          </div>
        {/if}
      </div>

      <div class="card pad moves">
        <MoveList moves={game.moves} {evals} current={variation.length ? -1 : ply} onselect={select} />
      </div>

      <div class="card pad explain">
        {#if currentMove && analysis}
          <ExplanationPanel
            move={currentMove}
            fenBefore={mainFen(currentMove.ply - 1)}
            explanation={explanations.get(currentMove.ply)}
            pending={pending.has(currentMove.ply)}
            failed={failed[currentMove.ply]}
            canExplain={!!app.meta?.has_provider && analysis.status !== 'running'}
            onexplain={explainNow}
            onshowline={showLine}
          />
        {:else if exploring}
          <p class="muted">You are exploring a line. Ask the coach about it below, or turn on Stockfish (space).</p>
        {:else if analysis?.key_moments.length}
          <p class="muted">
            Select a move, or jump between the {analysis.key_moments.length} key moments with ↑ ↓.
          </p>
        {:else if analysis}
          <p class="muted">Select a move to see how it was judged.</p>
        {:else}
          <p class="muted">Run the analysis to classify every move and find the key moments.</p>
        {/if}
      </div>

      <div class="card pad chatbox">
        <ChatPanel
          gameId={game.summary.id}
          fen={currentFen}
          ply={variation.length ? null : ply}
          {movePath}
          hasProvider={!!app.meta?.has_provider}
          onshapes={(s) => (toolShapes = s)}
        />
      </div>
    </section>
  </div>
{/if}

<style>
  .layout {
    display: grid;
    grid-template-columns: minmax(320px, min(calc(100vh - 150px), 680px)) minmax(340px, 1fr);
    gap: 1.2rem;
    align-items: start;
  }
  .left,
  .right {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    min-width: 0;
  }
  .boardrow {
    display: grid;
    grid-template-columns: 22px 1fr;
    gap: 6px;
  }
  .evalbar {
    height: 100%;
  }
  .player {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0 0 0 28px;
  }
  .player .dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1px solid var(--border);
  }
  .dot.white {
    background: #f4f2ec;
  }
  .dot.black {
    background: #2b2a28;
  }
  .spacer {
    flex: 1;
  }
  .acc {
    font-weight: 700;
  }
  .counts {
    display: flex;
    gap: 0.4rem;
    font-size: 0.8rem;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
    padding-left: 28px;
  }
  .explore {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.85rem;
    margin-left: 0.4rem;
  }
  .pad {
    padding: 0.7rem 0.9rem;
  }
  .info .title {
    font-size: 1.05rem;
  }
  .res {
    margin-left: 0.4rem;
  }
  .small {
    font-size: 0.85rem;
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.6rem;
  }
  .meter {
    height: 4px;
    background: var(--surface-2);
    border-radius: 2px;
    margin-top: 0.4rem;
    overflow: hidden;
  }
  .meter div {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }
  .analyse {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 0.6rem;
    margin-top: 0.7rem;
  }
  .analyse label {
    display: flex;
    flex-direction: column;
    font-size: 0.8rem;
    gap: 0.15rem;
  }
  .analyse label input[type='number'] {
    width: 6.5rem;
  }
  .analyse .check {
    flex-direction: row;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.9rem;
    padding-bottom: 0.45rem;
  }
  .done {
    margin-top: 0.5rem;
  }
  .review {
    margin-top: 0.6rem;
    padding-top: 0.6rem;
    border-top: 1px solid var(--border);
  }
  .review p {
    margin: 0 0 0.4rem;
  }
  .moves {
    max-height: 15rem;
    overflow: hidden;
    display: flex;
  }
  .moves :global(.moves) {
    flex: 1;
  }
  .chatbox {
    height: 30rem;
  }
  @media (max-width: 1000px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }
</style>
