// Runs the browser engine wrapper and the judging pipeline against the real
// Stockfish 19 lite WebAssembly build (as a Node child process).
import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { MoveEval, Score } from '../api/types';
import { browserEngine, setEngineWorkerFactory, type EngineWorker } from './engine';
import { parseGames, phase } from './game';
import { gameAccuracy, judge, keyMoments, moveAccuracy } from './judge';

const root = join(__dirname, '../../../..');

function nodeWorker(): EngineWorker {
  const child = spawn(process.execPath, [join(__dirname, '../../../node_modules/stockfish/bin/stockfish-19-lite-single.js')]);
  const w: EngineWorker = {
    onmessage: null,
    onerror: null,
    postMessage: (cmd) => child.stdin.write(cmd + '\n')
  };
  let buf = '';
  child.stdout.on('data', (d: Buffer) => {
    buf += d.toString();
    let i: number;
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i).trim();
      buf = buf.slice(i + 1);
      if (line) w.onmessage?.({ data: line });
    }
  });
  process.on('exit', () => child.kill());
  return w;
}

describe('browser engine pipeline', () => {
  setEngineWorkerFactory(nodeWorker);

  it('finds the mate in two', async () => {
    const a = await browserEngine().analyse('r2qkb1r/pp2nppp/3p4/2pNN1B1/2BnP3/3P4/PPP2PPP/R2bK2R w KQkq - 1 10', { depth: 10, multipv: 2 });
    expect(a.lines[0].score).toEqual({ kind: 'mate', value: 2 });
    expect(a.lines[0].pv_san[0]).toBe('Nf6+');
    expect(a.lines.length).toBe(2);
  }, 60000);

  it('scores from White’s side when Black is to move', async () => {
    const a = await browserEngine().analyse('3r2k1/5ppp/8/8/8/8/5PPP/6K1 b - - 0 1', { depth: 8 });
    expect(a.lines[0].score).toEqual({ kind: 'mate', value: -1 });
  }, 60000);

  it('analyses the Opera game end to end', async () => {
    const [g] = parseGames(readFileSync(join(root, 'fixtures/games/opera.pgn'), 'utf8'));
    const fens = [g.startFen, ...g.moves.map((m) => m.fen_after)];
    const res: { score: Score; best: string | null }[] = [];
    for (const f of fens) {
      const a = await browserEngine().analyse(f, { depth: 10 });
      const white = f.split(' ')[1] !== 'b';
      const score: Score = a.terminal === 'checkmate' ? { kind: 'mate', value: white ? -1 : 1 } : a.lines[0].score;
      res.push({ score, best: a.lines[0]?.pv_uci[0] ?? null });
    }
    const moves: MoveEval[] = g.moves.map((pm, i) => {
      const v = judge({ mover: pm.mover, prev: res[i].score, cur: res[i + 1].score, bestUci: res[i].best, playedUci: pm.uci, isBook: false });
      return {
        ply: pm.ply, mover: pm.mover, san: pm.san, uci: pm.uci, score: res[i + 1].score, depth: 10,
        best_uci: res[i].best, best_san: null, best_line_san: [], classification: v.classification,
        lichess_judgement: v.lichess, win_before: v.winBefore, win_after: v.winAfter, delta_wc: v.deltaWc,
        accuracy: moveAccuracy(v.winBefore, v.winAfter), phase: phase(pm.fen_before), is_key_moment: false
      };
    });
    expect(moves[32].classification).toBe('best');
    expect(moves[32].score).toEqual({ kind: 'mate', value: 1 });
    const acc = gameAccuracy(true, moves.map((m) => m.score));
    expect(acc.white!).toBeGreaterThan(acc.black!);
    expect(moves.filter((m) => m.mover === 'black' && ['mistake', 'blunder', 'inaccuracy'].includes(m.classification)).length).toBeGreaterThanOrEqual(2);
    expect(keyMoments(moves, 'black', '1-0').length).toBeGreaterThan(0);
  }, 180000);
});
