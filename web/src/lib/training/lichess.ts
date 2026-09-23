// Extra practice from Lichess's puzzle API (CC0 puzzles, CORS-enabled, no
// key), themed on the player's weakest motif.
import { START_FEN, playSan } from '$lib/chess';

/** chessgpt motif tag -> Lichess puzzle theme ("angle"). */
export const LICHESS_THEME: Record<string, string> = {
  hanging_piece: 'hangingPiece',
  fork: 'fork',
  pin: 'pin',
  skewer: 'skewer',
  discovered_attack: 'discoveredAttack',
  back_rank: 'backRankMate',
  mating_attack: 'mateIn2',
  trapped_piece: 'trappedPiece',
  promotion: 'promotion',
  king_safety: 'exposedKing',
  passed_pawn: 'advancedPawn'
};

export interface LichessPuzzle {
  id: string;
  fen: string;
  /** UCI moves, the solver's first. */
  solution: string[];
  rating: number;
  themes: string[];
}

/** Position after a space-separated SAN move list from the start. */
export function fenAfter(sanMoves: string): string | null {
  let fen = START_FEN;
  for (const san of sanMoves.trim().split(/\s+/).filter(Boolean)) {
    const p = playSan(fen, san);
    if (!p) return null;
    fen = p.fen;
  }
  return fen;
}

export async function nextLichessPuzzle(
  theme: string | null,
  difficulty: 'easiest' | 'easier' | 'normal' | 'harder' | 'hardest' = 'normal'
): Promise<LichessPuzzle> {
  const q = new URLSearchParams({ difficulty });
  if (theme) q.set('angle', theme);
  const res = await fetch(`https://lichess.org/api/puzzle/next?${q}`, { headers: { accept: 'application/json' } });
  if (!res.ok) throw new Error(`Lichess puzzles are unavailable right now (${res.status}).`);
  const d = (await res.json()) as {
    game: { pgn: string };
    puzzle: { id: string; rating: number; solution: string[]; themes: string[] };
  };
  const fen = fenAfter(d.game.pgn);
  if (!fen) throw new Error('Could not read the puzzle position.');
  return { id: d.puzzle.id, fen, solution: d.puzzle.solution, rating: d.puzzle.rating, themes: d.puzzle.themes };
}
