// Browser copy of crates/chess-core classify.rs, accuracy.rs and
// crates/coach key_moments.rs. Tested against the same Lichess fixtures.
import type { Classification, Judgement, MoveEval, Phase, Score, Side } from '../api/types';

const MULT = -0.00368208;
const CEIL = 1000;

export function winningChances(cp: number): number {
  return Math.max(-1, Math.min(1, 2 / (1 + Math.exp(MULT * cp)) - 1));
}

export function ceiledCp(s: Score): number {
  if (s.kind === 'mate') return s.value >= 0 ? CEIL : -CEIL;
  return Math.max(-CEIL, Math.min(CEIL, s.value));
}

export function winPercentCp(cp: number): number {
  return 50 + 50 * winningChances(Math.max(-CEIL, Math.min(CEIL, cp)));
}

/** Win% for `mover` from a White-POV score. */
export function winFor(s: Score, mover: Side): number {
  const w = winPercentCp(ceiledCp(s));
  return mover === 'white' ? w : 100 - w;
}

const pov = (s: Score, mover: Side): Score => (mover === 'white' ? s : { kind: s.kind, value: -s.value } as Score);

export function lichessJudgement(prev: Score, cur: Score, mover: Side): Judgement | null {
  if (prev.kind === 'cp' && cur.kind === 'cp') {
    const d = winningChances(cur.value) - winningChances(prev.value);
    const delta = mover === 'white' ? -d : d;
    if (delta >= 0.3) return 'blunder';
    if (delta >= 0.2) return 'mistake';
    if (delta >= 0.1) return 'inaccuracy';
    return null;
  }
  const p = pov(prev, mover);
  const c = pov(cur, mover);
  let seq: 'created' | 'lost' | null = null;
  if (p.kind === 'cp' && c.kind === 'mate' && c.value < 0) seq = 'created';
  else if (p.kind === 'mate' && p.value > 0 && c.kind === 'cp') seq = 'lost';
  else if (p.kind === 'mate' && p.value > 0 && c.kind === 'mate' && c.value < 0) seq = 'lost';
  if (!seq) return null;
  const prevCp = p.kind === 'cp' ? p.value : 0;
  const curCp = c.kind === 'cp' ? c.value : 0;
  if (seq === 'created') return prevCp < -999 ? 'inaccuracy' : prevCp < -700 ? 'mistake' : 'blunder';
  return curCp > 999 ? 'inaccuracy' : curCp > 700 ? 'mistake' : 'blunder';
}

export const MISSED_WIN_FROM = 85;
export const EXCELLENT_BELOW = 0.02;

export function judge(o: {
  mover: Side;
  prev: Score;
  cur: Score;
  bestUci: string | null;
  playedUci: string;
  isBook: boolean;
}): { classification: Classification; lichess: Judgement | null; winBefore: number; winAfter: number; deltaWc: number } {
  const winBefore = winFor(o.prev, o.mover);
  const winAfter = winFor(o.cur, o.mover);
  const deltaWc = (winBefore - winAfter) / 50;
  const lj = lichessJudgement(o.prev, o.cur, o.mover);
  let classification: Classification;
  if (o.isBook && !lj) classification = 'book';
  else if (lj) {
    const p = pov(o.prev, o.mover);
    const hadWin = winBefore >= MISSED_WIN_FROM || (p.kind === 'mate' && p.value > 0);
    if ((lj === 'mistake' || lj === 'blunder') && hadWin && winAfter < MISSED_WIN_FROM) classification = 'missed_win';
    else classification = lj;
  } else if (o.bestUci && o.bestUci === o.playedUci) classification = 'best';
  else if (deltaWc < EXCELLENT_BELOW) classification = 'excellent';
  else classification = 'good';
  return { classification, lichess: lj, winBefore, winAfter, deltaWc };
}

export function moveAccuracy(before: number, after: number): number {
  if (after >= before) return 100;
  const raw = 103.1668100711649 * Math.exp(-0.04354415386753951 * (before - after)) - 3.166924740191411;
  return Math.max(0, Math.min(100, raw + 1));
}

function stdDev(xs: number[]): number {
  if (!xs.length) return 0;
  const m = xs.reduce((a, b) => a + b, 0) / xs.length;
  return Math.sqrt(xs.reduce((a, x) => a + (x - m) ** 2, 0) / xs.length);
}

/** Lichess game accuracy from White-POV scores after each ply. */
export function gameAccuracy(whiteStarts: boolean, scores: Score[]): { white: number | null; black: number | null } {
  const all = [15, ...scores.map(ceiledCp)].map(winPercentCp);
  if (all.length < 2) return { white: null, black: null };
  const ws = Math.max(2, Math.min(8, Math.floor(scores.length / 10)));
  const windows: number[][] = [];
  const firstLen = Math.min(ws, all.length);
  for (let i = 0; i < Math.max(0, firstLen - 2); i++) windows.push(all.slice(0, firstLen));
  if (all.length >= ws) for (let i = 0; i + ws <= all.length; i++) windows.push(all.slice(i, i + ws));
  else windows.push(all);
  const weights = windows.map((w) => Math.max(0.5, Math.min(12, stdDev(w))));
  const per: { acc: number; w: number; white: boolean }[] = [];
  for (let i = 0; i + 1 < all.length && i < weights.length; i++) {
    const white = (i % 2 === 0) === whiteStarts;
    const acc = white ? moveAccuracy(all[i], all[i + 1]) : moveAccuracy(100 - all[i], 100 - all[i + 1]);
    per.push({ acc, w: weights[i], white });
  }
  const colour = (white: boolean) => {
    const xs = per.filter((p) => p.white === white);
    if (!xs.length) return null;
    const tw = xs.reduce((a, p) => a + p.w, 0);
    const weighted = xs.reduce((a, p) => a + p.acc * p.w, 0) / tw;
    const harmonic = xs.length / xs.reduce((a, p) => a + 1 / Math.max(1, p.acc), 0);
    return (weighted + harmonic) / 2;
  };
  return { white: colour(true), black: colour(false) };
}

const severity = (c: Classification) =>
  c === 'blunder' || c === 'missed_win' ? 3 : c === 'mistake' ? 2 : c === 'inaccuracy' ? 1 : 0;
const isErr = (c: Classification) => severity(c) > 0;
export const ALREADY_LOST = 10;

export function turningPoint(moves: MoveEval[], result: string): number | null {
  const loser: Side | null = result === '1-0' ? 'black' : result === '0-1' ? 'white' : null;
  if (!loser) return null;
  const loserWin = (m: MoveEval) => (m.mover === loser ? m.win_after : 100 - m.win_after);
  let last = -1;
  moves.forEach((m, i) => {
    if (loserWin(m) > 50) last = i;
  });
  const idx = last >= 0 ? (last + 1 < moves.length ? last + 1 : -1) : 0;
  return idx >= 0 && moves[idx] ? moves[idx].ply : null;
}

export function keyMoments(moves: MoveEval[], userSide: Side | null, result: string, max = 8): number[] {
  const weight = (m: MoveEval) => {
    const base = severity(m.classification) * Math.max(0, m.delta_wc);
    return userSide && userSide === m.mover ? base * 1.5 : base;
  };
  const scored: [number, number, Side][] = moves
    .filter((m) => isErr(m.classification) && m.win_before >= ALREADY_LOST)
    .map((m) => [weight(m), m.ply, m.mover]);
  const tp = turningPoint(moves, result);
  const tpm = tp !== null ? moves.find((m) => m.ply === tp) : undefined;
  if (tpm && !scored.some((s) => s[1] === tp) && tpm.delta_wc > 0.05 && tpm.win_before >= ALREADY_LOST)
    scored.push([Math.max(0.5, weight(tpm)), tpm.ply, tpm.mover]);
  for (const phase of ['opening', 'middlegame', 'endgame'] as Phase[]) {
    const c = moves.filter((m) => m.phase === phase && m.delta_wc > 0.1 && m.win_before >= ALREADY_LOST);
    if (!c.length) continue;
    const m = c.reduce((a, b) => (b.delta_wc > a.delta_wc ? b : a));
    if (!scored.some((s) => s[1] === m.ply)) scored.push([Math.max(0.3, weight(m)), m.ply, m.mover]);
  }
  scored.sort((a, b) => b[0] - a[0]);
  const chosen: [number, Side][] = [];
  for (const [, ply, side] of scored) {
    if (chosen.length >= max) break;
    if (chosen.some(([p, s]) => s === side && Math.abs(p - ply) <= 2)) continue;
    chosen.push([ply, side]);
  }
  return chosen.map((c) => c[0]).sort((a, b) => a - b);
}
