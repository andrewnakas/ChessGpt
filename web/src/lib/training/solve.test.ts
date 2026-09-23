import { describe, expect, it } from 'vitest';
import { START_FEN } from '$lib/chess';
import { fenAfter } from './lichess';
import { step } from './solve';

describe('puzzle solving', () => {
  it('walks a multi-move solution', () => {
    const sol = ['e2e4', 'e7e5', 'g1f3'];
    const s1 = step(START_FEN, sol, 0, 'e2e4');
    expect(s1.kind).toBe('continue');
    if (s1.kind !== 'continue') return;
    expect(s1.reply).toBe('e7e5');
    expect(step(s1.fen, sol, s1.next, 'g1f3').kind).toBe('solved');
    expect(step(START_FEN, sol, 0, 'd2d4')).toEqual({ kind: 'wrong', expected: 'e2e4' });
  });

  it('single-move puzzles solve at once', () => {
    expect(step(START_FEN, ['g1f3'], 0, 'g1f3').kind).toBe('solved');
  });

  it('reads a Lichess SAN move list', () => {
    expect(fenAfter('e4 e5 Nf3')).toBe('rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2');
    expect(fenAfter('e4 Ke7 Kxx')).toBeNull();
  });
});
