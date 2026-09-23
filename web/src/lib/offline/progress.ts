// Progress for browser mode: the same shape the server's /api/progress
// returns, from games stored in this browser (no motif tags here).
import type { GameAnalysis, GameSummary, Phase, Progress, ProgressGame } from '../api/types';

const isError = (c: string) => c === 'mistake' || c === 'blunder' || c === 'missed_win';

function userScore(result: string, side: 'white' | 'black'): number | null {
  const w = result === '1-0' ? 1 : result === '0-1' ? 0 : result === '1/2-1/2' ? 0.5 : null;
  return w === null ? null : side === 'white' ? w : 1 - w;
}

// chess_core::rating::CALIBRATION and its residual spread.
const CAL_A = 1280.145;
const CAL_B = 0.22304;
const RESIDUAL_SD = 183;
export const ROLLING_GAMES = 20;

/** Rating and ± margin from the last 20 per-game estimates, corrected for
 * shrinkage (chess_core::rating::rolling). */
export function rolling(estimates: number[]): [number, number] | null {
  const r = estimates.slice(-ROLLING_GAMES);
  if (!r.length) return null;
  const m = r.reduce((a, b) => a + b, 0) / r.length;
  const rating = Math.min(3000, Math.max(400, (m - CAL_A) / CAL_B));
  const margin = RESIDUAL_SD / CAL_B / Math.sqrt(r.length);
  return [Math.round(rating / 25) * 25, Math.round(margin / 25) * 25];
}

/** Linear performance rating (chess_core::rating::performance). */
export function performance(results: [number, number][]): number | null {
  if (!results.length) return null;
  const n = results.length;
  const avg = results.reduce((s, [r]) => s + r, 0) / n;
  const net = results.reduce((s, [, x]) => s + 2 * x - 1, 0);
  return Math.round(Math.min(3500, Math.max(100, avg + (400 * net) / n)));
}

export function progressFrom(stored: { summary: GameSummary; analysis: GameAnalysis | null }[]): Progress {
  const done = stored
    .filter((g) => g.analysis?.status === 'done' && g.summary.user_side)
    .sort((a, b) => (a.summary.date ?? '').localeCompare(b.summary.date ?? '') || a.summary.imported_at - b.summary.imported_at);
  const phaseAgg = new Map<Phase, { moves: number; errors: number }>();
  const games: ProgressGame[] = done.map(({ summary: s, analysis: a }) => {
    const side = s.user_side!;
    const mine = a!.moves.filter((m) => m.mover === side);
    for (const m of mine) {
      const p = phaseAgg.get(m.phase) ?? { moves: 0, errors: 0 };
      p.moves += 1;
      if (isError(m.classification)) p.errors += 1;
      phaseAgg.set(m.phase, p);
    }
    const white = side === 'white';
    return {
      game_id: s.id,
      date: s.date,
      imported_at: s.imported_at,
      user_side: side,
      opponent: white ? s.black : s.white,
      user_elo: white ? s.white_elo : s.black_elo,
      opponent_elo: white ? s.black_elo : s.white_elo,
      score: userScore(s.result, side),
      time_control: s.time_control,
      accuracy: white ? a!.white_accuracy : a!.black_accuracy,
      estimate: white ? a!.white_estimate : a!.black_estimate,
      moves: mine.length,
      errors: mine.filter((m) => isError(m.classification)).length
    };
  });
  const rated = games.filter((g) => g.opponent_elo != null && g.score != null).map((g) => [g.opponent_elo!, g.score!] as [number, number]);
  const phases = [...phaseAgg].map(([phase, v]) => ({
    phase,
    moves: v.moves,
    errors: v.errors,
    per_100_moves: v.moves ? Math.round((v.errors * 1000) / v.moves) / 10 : 0
  }));
  const roll = rolling(games.flatMap((g) => (g.estimate == null ? [] : [g.estimate])));
  return {
    games,
    rolling_estimate: roll?.[0] ?? null,
    estimate_margin: roll?.[1] ?? null,
    performance: performance(rated.slice(-20)),
    user_moves: phases.reduce((s, p) => s + p.moves, 0),
    motifs: [],
    phases
  };
}
