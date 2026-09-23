import { describe, expect, it } from 'vitest';
import { choose, styleFor } from './bot';

function seeded(seed: number) {
  return () => {
    seed = (seed * 1103515245 + 12345) % 2 ** 31;
    return seed / 2 ** 31;
  };
}

describe('sparring bot', () => {
  it('interpolates styles and gets stronger with rating', () => {
    const a = styleFor(1000);
    const b = styleFor(2000);
    expect(a.depth).toBeLessThan(b.depth);
    expect(a.blunder).toBeGreaterThan(b.blunder);
    expect(styleFor(3000)).toEqual(styleFor(2600));
  });

  it('strong bots almost always play the best move, weak ones spread out', () => {
    const cands = [
      { uci: 'e2e4', win: 60 },
      { uci: 'd2d4', win: 55 },
      { uci: 'a2a3', win: 40 }
    ];
    const legal = cands.map((c) => c.uci);
    const count = (elo: number) => {
      const r = seeded(7);
      let best = 0;
      for (let i = 0; i < 1000; i++) if (choose(cands, legal, styleFor(elo), r) === 'e2e4') best++;
      return best;
    };
    expect(count(2600)).toBeGreaterThan(990);
    expect(count(800)).toBeLessThan(700);
  });
});
