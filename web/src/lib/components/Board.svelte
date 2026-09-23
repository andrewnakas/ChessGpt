<script lang="ts">
  import { Chessground } from '@lichess-org/chessground';
  import type { Api } from '@lichess-org/chessground/api';
  import type { DrawShape } from '@lichess-org/chessground/draw';
  import type { Key } from '@lichess-org/chessground/types';
  import { untrack } from 'svelte';
  import { dests, inCheck, turn } from '$lib/chess';

  interface Props {
    fen: string;
    orientation?: 'white' | 'black';
    lastMove?: [Key, Key] | null;
    shapes?: DrawShape[];
    interactive?: boolean;
    onmove?: (orig: Key, dest: Key) => void;
  }
  let { fen, orientation = 'white', lastMove = null, shapes = [], interactive = true, onmove }: Props = $props();

  let el: HTMLDivElement;
  let cg: Api | undefined;

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
        movable: {
          ...config().movable,
          events: { after: (orig: Key, dest: Key) => onmove?.(orig, dest) }
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
  .board :global(cg-wrap) {
    width: 100%;
    height: 100%;
  }
</style>
