import { describe, expect, it } from 'vitest';
import { START_FEN, dests, fmtScore, numberedLine, playMove, playSan, playUci, whiteWin } from './chess';

describe('chess helpers', () => {
  it('generates start position dests', () => {
    const d = dests(START_FEN);
    expect(d.get('e2')).toEqual(['e3', 'e4']);
    expect([...d.values()].flat().length).toBe(20);
  });

  it('plays moves in all notations', () => {
    const a = playMove(START_FEN, 'e2', 'e4')!;
    expect(a.san).toBe('e4');
    const b = playSan(a.fen, 'e5')!;
    expect(b.uci).toBe('e7e5');
    const c = playUci(b.fen, 'g1f3')!;
    expect(c.san).toBe('Nf3');
    expect(playMove(START_FEN, 'e2', 'e5')).toBeNull();
  });

  it('uses standard castling UCI both ways', () => {
    const fen = 'r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1';
    expect(playMove(fen, 'e1', 'h1')?.uci).toBe('e1g1');
    expect(playSan(fen, 'O-O-O')?.uci).toBe('e1c1');
    expect(playUci(fen, 'e1g1')?.san).toBe('O-O');
  });

  it('auto-queens promotions', () => {
    const r = playMove('8/P7/8/8/8/8/8/k6K w - - 0 1', 'a7', 'a8')!;
    expect(r.san).toBe('a8=Q+');
  });

  it('numbers lines from either side', () => {
    expect(numberedLine(START_FEN, ['e4', 'e5', 'Nf3'])).toBe('1. e4 e5 2. Nf3');
    const b = playSan(START_FEN, 'e4')!.fen;
    expect(numberedLine(b, ['e5', 'Nf3'])).toBe('1... e5 2. Nf3');
  });

  it('matches the server win% model', () => {
    expect(whiteWin({ kind: 'cp', value: 0 })).toBe(50);
    expect(whiteWin({ kind: 'mate', value: 3 })).toBeCloseTo(97.54, 1);
    expect(fmtScore({ kind: 'cp', value: -35 })).toBe('-0.3');
    expect(fmtScore({ kind: 'mate', value: -2 })).toBe('#-2');
  });
});
