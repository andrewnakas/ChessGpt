// Stepping through a puzzle solution: the player's moves alternate with
// scripted replies.
import { playUci } from '$lib/chess';

export type Step =
  | { kind: 'wrong'; expected: string }
  | { kind: 'continue'; fen: string; reply: string | null; next: number }
  | { kind: 'solved'; fen: string };

/** The player played `uci` at solution index `at` (always the player's turn). */
export function step(fen: string, solution: string[], at: number, uci: string): Step {
  const expected = solution[at];
  // Promotion to a queen is the board default; accept it for any promotion
  // that the solution spells the same way.
  if (!expected || uci.slice(0, 4) !== expected.slice(0, 4) || (expected.length > 4 && uci.length > 4 && uci[4] !== expected[4])) {
    return { kind: 'wrong', expected };
  }
  const mine = playUci(fen, expected);
  if (!mine) return { kind: 'wrong', expected };
  const reply = solution[at + 1];
  if (!reply) return { kind: 'solved', fen: mine.fen };
  const theirs = playUci(mine.fen, reply);
  if (!theirs) return { kind: 'solved', fen: mine.fen };
  if (at + 2 >= solution.length) return { kind: 'solved', fen: theirs.fen };
  return { kind: 'continue', fen: theirs.fen, reply, next: at + 2 };
}
