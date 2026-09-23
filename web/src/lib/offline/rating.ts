// Port of chess_core::rating: estimated playing strength from one side's
// move quality in one game. Keep COEF in sync with crates/chess-core/src/rating.rs.
import type { MoveEval } from '../api/types';

export const COEF = [
  2880.071714, -20.878259, -8.014965, -283.347639, -467.532986, -439.788865, -93.947935, 19.652107, 1.372252,
  -1.961966, -114.731339, 169.704593
];

const MIN_MOVES = 10;

const mean = (v: number[]) => (v.length ? v.reduce((a, b) => a + b, 0) / v.length : null);

export function features(moves: MoveEval[], baseSeconds: number | null): number[] | null {
  if (moves.length < MIN_MOVES) return null;
  const n = moves.length;
  const acc = mean(moves.map((m) => m.accuracy))!;
  const rate = (j: string) => moves.filter((m) => m.lichess_judgement === j).length / n;
  const loss = mean(moves.map((m) => Math.min(50, Math.max(0, m.win_before - m.win_after))))!;
  const phaseAcc = (p: string) => mean(moves.filter((m) => m.phase === p).map((m) => m.accuracy)) ?? acc;
  const base = Math.max(1, baseSeconds ?? 300);
  return [
    1,
    acc,
    (acc * acc) / 100,
    rate('blunder'),
    rate('mistake'),
    rate('inaccuracy'),
    loss,
    phaseAcc('opening'),
    phaseAcc('middlegame'),
    phaseAcc('endgame'),
    Math.log(base),
    Math.log(n)
  ];
}

export function estimate(moves: MoveEval[], baseSeconds: number | null): number | null {
  const f = features(moves, baseSeconds);
  if (!f) return null;
  const y = f.reduce((s, x, i) => s + x * COEF[i], 0);
  return Math.round(Math.min(3000, Math.max(400, y)));
}

export function baseSeconds(timeControl: string | null): number | null {
  const n = Number.parseInt((timeControl ?? '').split('+')[0], 10);
  return Number.isFinite(n) ? n : null;
}
