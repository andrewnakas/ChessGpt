<script lang="ts">
  import type { Explanation, MoveEval } from '$lib/api/types';
  import { CLASS_GLYPH, CLASS_LABEL, fmtScore, isError, moveNumber, numberedLine, tagLabel } from '$lib/chess';

  interface Props {
    move: MoveEval;
    fenBefore: string;
    explanation?: Explanation;
    pending: boolean;
    failed?: string;
    canExplain: boolean;
    onexplain: () => void;
    /** Show a SAN line from the position before the move. */
    onshowline: (sans: string[]) => void;
  }
  let { move, fenBefore, explanation, pending, failed, canExplain, onexplain, onshowline }: Props = $props();

  const moverWinBefore = $derived(move.win_before.toFixed(0));
  const moverWinAfter = $derived(move.win_after.toFixed(0));
</script>

<div class="panel">
  <div class="summary">
    <span class="big c-{move.classification}">{moveNumber(fenBefore)} {move.san}{CLASS_GLYPH[move.classification]}</span>
    <span class="chip c-{move.classification}">{CLASS_LABEL[move.classification]}</span>
    <span class="muted small">
      {move.mover === 'white' ? 'White' : 'Black'}'s winning chances {moverWinBefore}% → {moverWinAfter}% · eval {fmtScore(move.score)}
    </span>
  </div>
  {#if move.best_san && move.best_san !== move.san}
    <div class="best">
      Engine's choice:
      <button class="linkish mono" onclick={() => onshowline(move.best_line_san.slice(0, 8))}>
        {numberedLine(fenBefore, move.best_line_san.slice(0, 6))}
      </button>
    </div>
  {/if}

  {#if explanation}
    <div class="expl">
      <h3>{explanation.headline}</h3>
      <p>{explanation.why_it_matters}</p>
      {#if explanation.better_move}
        <div class="better">
          <strong>Better:</strong>
          <button class="linkish mono" onclick={() => onshowline(explanation!.better_move!.line_san)}>
            {numberedLine(fenBefore, explanation.better_move.line_san)}
          </button>
          {#if explanation.better_move.reason}<div>{explanation.better_move.reason}</div>{/if}
        </div>
      {/if}
      {#if explanation.takeaway}
        <div class="takeaway"><span aria-hidden="true">💡</span> {explanation.takeaway}</div>
      {/if}
      <div class="foot">
        {#each explanation.concept_tags as t}<span class="chip">{tagLabel(t)}</span>{/each}
        <span class="spacer"></span>
        {#if explanation.verification.status === 'ok'}
          <span class="verified" title="Every move mentioned was checked against Stockfish">✓ engine-checked</span>
        {:else}
          <span class="partial" title={explanation.verification.issues.join('\n')}>⚠ partly verified</span>
        {/if}
        <span class="muted tiny">{explanation.model}</span>
      </div>
      {#if explanation.verification.unverified_moves.length}
        <div class="muted tiny">Not engine-backed: {explanation.verification.unverified_moves.join(', ')}</div>
      {/if}
    </div>
  {:else if pending}
    <div class="muted"><span class="spinner"></span> The coach is explaining this moment…</div>
  {:else if failed}
    <div class="error small">Explanation failed: {failed}</div>
    {#if canExplain}<button onclick={onexplain}>Try again</button>{/if}
  {:else if canExplain}
    <button class={isError(move.classification) || move.is_key_moment ? 'primary' : ''} onclick={onexplain}>
      Explain this move
    </button>
  {/if}
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .summary {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.5rem;
  }
  .big {
    font-size: 1.15rem;
    font-weight: 700;
    font-family: var(--mono);
  }
  .small {
    font-size: 0.85rem;
  }
  .tiny {
    font-size: 0.75rem;
  }
  .best {
    font-size: 0.9rem;
  }
  .linkish {
    background: none;
    border: none;
    padding: 0;
    color: var(--link);
    text-align: left;
  }
  .linkish:hover {
    text-decoration: underline;
  }
  .expl h3 {
    margin: 0.2rem 0;
    font-size: 1.05rem;
  }
  .expl p {
    margin: 0.3rem 0;
  }
  .better {
    margin: 0.4rem 0;
    padding: 0.5rem 0.6rem;
    border-left: 3px solid var(--c-best);
    background: var(--surface-2);
    border-radius: 0 8px 8px 0;
  }
  .takeaway {
    margin: 0.4rem 0;
    padding: 0.5rem 0.6rem;
    border-left: 3px solid var(--accent);
    background: var(--surface-2);
    border-radius: 0 8px 8px 0;
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    margin-top: 0.3rem;
  }
  .spacer {
    flex: 1;
  }
  .verified {
    color: var(--ok);
    font-size: 0.8rem;
  }
  .partial {
    color: var(--warn);
    font-size: 0.8rem;
    cursor: help;
  }
</style>
