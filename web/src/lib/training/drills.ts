// Technique drills: grading for the question formats. Sets are built by the
// server (crates/coach/src/drills.rs).
import type { DrillItem, DrillKind, DrillQuiz } from '$lib/api/types';

/** Concept tags that can be drilled (crates/coach/src/bank.rs TECHNIQUES). */
export const DRILLABLE = [
  'hanging_piece',
  'fork',
  'pin',
  'skewer',
  'discovered_attack',
  'back_rank',
  'mating_attack',
  'trapped_piece',
  'promotion',
  'king_safety',
  'passed_pawn',
  'deflection',
  'decoy',
  'endgame_technique'
];

export function drillableTag(tags: string[]): string | null {
  return tags.find((t) => DRILLABLE.includes(t)) ?? null;
}

export const KIND_LABEL: Record<DrillKind, string> = {
  spot: 'Spot it',
  find: 'Find it',
  defend: 'Stop it',
  playout: 'Play it out'
};

/** Same move, ignoring a promotion suffix the board may add or omit. */
function sameMove(a: string, b: string): boolean {
  return a.slice(0, 4) === b.slice(0, 4) && (a.length < 5 || b.length < 5 || a[4] === b[4]);
}

/** Is a first move one of the accepted defences? */
export function accepts(accept: string[], uci: string): boolean {
  return accept.some((a) => sameMove(a, uci));
}

export function gradeTap(quiz: DrillQuiz, square: string): boolean {
  return quiz.answer_squares.includes(square);
}

export function gradeChoice(quiz: DrillQuiz, choice: number): boolean {
  return quiz.answer_choice === choice;
}

/** Solved / total per kind, in drill order. */
export function tally(items: DrillItem[]): { kind: DrillKind; solved: number; total: number }[] {
  const out: { kind: DrillKind; solved: number; total: number }[] = [];
  for (const it of items) {
    let row = out.find((r) => r.kind === it.kind);
    if (!row) out.push((row = { kind: it.kind, solved: 0, total: 0 }));
    row.total += 1;
    if (it.solved) row.solved += 1;
  }
  return out;
}
