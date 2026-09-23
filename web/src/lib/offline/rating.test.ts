import { describe, expect, it } from 'vitest';
import type { MoveEval } from '../api/types';
import { baseSeconds, estimate } from './rating';

function move(acc: number, j: string | null = null): MoveEval {
  return { accuracy: acc, win_before: 50, win_after: 50 - (100 - acc) / 4, lichess_judgement: j, phase: 'middlegame' } as unknown as MoveEval;
}

describe('rating', () => {
  it('matches the Rust model on the same inputs', () => {
    // Same fixture as chess_core::rating::tests::better_play_rates_higher.
    const strong = Array.from({ length: 30 }, () => move(95));
    const weak = Array.from({ length: 30 }, () => move(70));
    weak[3] = move(70, 'blunder');
    weak[9] = move(70, 'blunder');
    expect(estimate(strong, 300)).toBe(1790);
    expect(estimate(weak, 300)).toBe(1560);
    expect(estimate(strong.slice(0, 9), 300)).toBeNull();
    expect(baseSeconds('600+5')).toBe(600);
  });
});
