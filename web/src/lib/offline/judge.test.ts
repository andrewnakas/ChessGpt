import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { gameAccuracy, judge, lichessJudgement, moveAccuracy, winPercentCp, winningChances } from './judge';
import { bookInfo, parseGames, phase } from './game';

const root = join(__dirname, '../../../..');

describe('browser judge matches chess-core', () => {
  it('win% and accuracy', () => {
    expect(winPercentCp(0)).toBe(50);
    expect(winPercentCp(5000)).toBe(winPercentCp(1000));
    expect(moveAccuracy(50, 50)).toBe(100);
    const a = moveAccuracy(50, 40);
    expect(a > 60 && a < 70).toBe(true);
  });

  it('exact thresholds', () => {
    const find = (t: number) => {
      for (let c = 0; c > -3000; c--) if (winningChances(0) - winningChances(c) >= t) return c;
      return 0;
    };
    for (const [t, j] of [[0.1, 'inaccuracy'], [0.2, 'mistake'], [0.3, 'blunder']] as const) {
      const c = find(t);
      expect(lichessJudgement({ kind: 'cp', value: 0 }, { kind: 'cp', value: c }, 'white')).toBe(j);
      expect(lichessJudgement({ kind: 'cp', value: 0 }, { kind: 'cp', value: c + 1 }, 'white')).not.toBe(j);
    }
    expect(lichessJudgement({ kind: 'cp', value: -1200 }, { kind: 'mate', value: -4 }, 'white')).toBe('inaccuracy');
    expect(lichessJudgement({ kind: 'mate', value: 2 }, { kind: 'mate', value: 6 }, 'white')).toBeNull();
    expect(judge({ mover: 'white', prev: { kind: 'cp', value: 900 }, cur: { kind: 'cp', value: 0 }, bestUci: 'e2e4', playedUci: 'a2a3', isBook: false }).classification).toBe('missed_win');
  });

  it('reproduces every Lichess annotation in the fixtures', () => {
    const dir = join(root, 'fixtures/lichess');
    let annotated = 0;
    for (const f of readdirSync(dir).filter((f) => f.endsWith('.pgn'))) {
      const [g] = parseGames(readFileSync(join(dir, f), 'utf8'));
      for (let i = 1; i < g.moves.length; i++) {
        const prev = g.moves[i - 1].pgn_eval;
        const cur = g.moves[i].pgn_eval;
        if (!prev || !cur) continue;
        const theirs = ({ 6: 'inaccuracy', 2: 'mistake', 4: 'blunder' } as Record<number, string>)[g.moves[i].nag ?? 0] ?? null;
        expect(lichessJudgement(prev, cur, g.moves[i].mover), `${f} ply ${g.moves[i].ply}`).toBe(theirs);
        if (theirs) annotated++;
      }
    }
    expect(annotated).toBeGreaterThanOrEqual(10);
  });

  it('parses the Opera game, finds the book and phase', async () => {
    const [g] = parseGames(readFileSync(join(root, 'fixtures/games/opera.pgn'), 'utf8'));
    expect(g.moves.length).toBe(33);
    expect(g.moves[22].uci).toBe('e1c1');
    const fakeFetch = (async (url: string) =>
      new Response(readFileSync(join(root, 'crates/chess-core/data', url), 'utf8'))) as unknown as typeof fetch;
    const { flags, opening } = await bookInfo(g.moves, fakeFetch);
    expect(flags[0]).toBe(true);
    expect(opening?.name).toContain('Philidor');
    expect(phase(g.moves[32].fen_after)).toBe('endgame');
    const acc = gameAccuracy(true, g.moves.map(() => ({ kind: 'cp', value: 15 })));
    expect(acc.white! > 99).toBe(true);
  });
});
