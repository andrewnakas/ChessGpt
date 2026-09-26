<script lang="ts">
  import { Chessground } from '@lichess-org/chessground';
  import type { Api } from '@lichess-org/chessground/api';
  import type { DrawShape } from '@lichess-org/chessground/draw';
  import type { Key } from '@lichess-org/chessground/types';
  import { untrack } from 'svelte';
  import { dests, inCheck, isPromotion, turn, type PromotionRole } from '$lib/chess';

  interface Props {
    fen: string;
    orientation?: 'white' | 'black';
    lastMove?: [Key, Key] | null;
    shapes?: DrawShape[];
    interactive?: boolean;
    /** `promotion` is set when a pawn reached the last rank (chosen in a picker). */
    onmove?: (orig: Key, dest: Key, promotion?: PromotionRole) => void;
    /** A square was clicked (also when the board is not interactive). */
    onselect?: (square: Key) => void;
  }
  let { fen, orientation = 'white', lastMove = null, shapes = [], interactive = true, onmove, onselect }: Props = $props();

  let el: HTMLDivElement;
  let cg: Api | undefined;
  // A pawn move to the last rank waits here for the piece to be picked.
  let promoting = $state<{ orig: Key; dest: Key; color: 'white' | 'black' } | null>(null);
  const ROLES: PromotionRole[] = ['queen', 'knight', 'rook', 'bishop'];

  function moved(orig: Key, dest: Key) {
    if (isPromotion(fen, orig, dest)) {
      promoting = { orig, dest, color: turn(fen) };
      return;
    }
    onmove?.(orig, dest);
  }

  function promote(role: PromotionRole | null) {
    const p = promoting;
    promoting = null;
    if (p && role) onmove?.(p.orig, p.dest, role);
    else cg?.set(config()); // cancelled: put the pawn back
  }

  function config() {
    const color = turn(fen);
    return {
      fen,
      orientation,
      turnColor: color,
      check: inCheck(fen) ? color : false,
      lastMove: lastMove ?? undefined,
      movable: {
        free: false,
        color: interactive ? color : undefined,
        dests: interactive ? dests(fen) : new Map(),
        showDests: true
      },
      drawable: { autoShapes: shapes }
    } as const;
  }

  $effect(() => {
    untrack(() => {
      cg = Chessground(el, {
        ...config(),
        coordinates: true,
        animation: { enabled: true, duration: 180 },
        highlight: { lastMove: true, check: true },
        premovable: { enabled: false },
        events: { select: (key: Key) => onselect?.(key) },
        movable: {
          ...config().movable,
          events: { after: (orig: Key, dest: Key) => moved(orig, dest) }
        },
        drawable: { enabled: true, visible: true, autoShapes: shapes }
      });
    });
    return () => cg?.destroy();
  });

  $effect(() => {
    // Re-apply whenever any prop changes.
    const c = config();
    cg?.set(c);
    cg?.setAutoShapes(shapes);
  });
</script>

<div class="board-wrap">
  <div class="board" bind:this={el}></div>
  {#if promoting}
    <div class="promo" role="dialog" aria-label="Promote to">
      <button class="scrim" aria-label="Cancel" onclick={() => promote(null)}></button>
      <div class="choices cg-wrap">
        {#each ROLES as r}
          <button class="piece-btn" title={r} aria-label="Promote to {r}" onclick={() => promote(r)}>
            <piece class="{promoting.color} {r}"></piece>
          </button>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .board-wrap {
    width: 100%;
    aspect-ratio: 1 / 1;
    position: relative;
  }
  .board {
    position: absolute;
    inset: 0;
  }
  .promo {
    position: absolute;
    inset: 0;
    z-index: 5;
    display: grid;
    place-items: center;
  }
  .scrim {
    position: absolute;
    inset: 0;
    background: rgb(0 0 0 / 0.35);
    border: 0;
    border-radius: 0;
    padding: 0;
  }
  .choices {
    position: relative;
    display: flex;
    gap: 0.4rem;
    background: var(--surface);
    padding: 0.5rem;
    border-radius: 10px;
    box-shadow: 0 6px 24px rgb(0 0 0 / 0.3);
  }
  .piece-btn {
    width: 4.2rem;
    height: 4.2rem;
    padding: 0;
    position: relative;
  }
  .piece-btn piece {
    position: absolute;
    inset: 6%;
    width: auto;
    height: auto;
    background-size: cover;
  }
  .board :global(cg-wrap) {
    width: 100%;
    height: 100%;
  }
</style>
