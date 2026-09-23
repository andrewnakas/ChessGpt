// Board-side chess logic via chessops. The server stays the source of truth
// for evaluations; this only handles legal moves and notation.
import { Chess } from 'chessops/chess';
import { chessgroundDests } from 'chessops/compat';
import { makeFen, parseFen } from 'chessops/fen';
import { makeSan, parseSan } from 'chessops/san';
import { makeUci, parseSquare, parseUci } from 'chessops/util';
import type { Key } from '@lichess-org/chessground/types';
import type { Classification, Score } from './api/types';

export const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';

export function position(fen: string): Chess | null {
  const setup = parseFen(fen.trim());
  if (setup.isErr) return null;
  const pos = Chess.fromSetup(setup.value);
  return pos.isOk ? pos.value : null;
}

export function isValidFen(fen: string): boolean {
  return position(fen) !== null;
}

export function dests(fen: string): Map<Key, Key[]> {
  const pos = position(fen);
  return pos ? (chessgroundDests(pos) as Map<Key, Key[]>) : new Map();
}

export function turn(fen: string): 'white' | 'black' {
  return fen.split(' ')[1] === 'b' ? 'black' : 'white';
}

export function inCheck(fen: string): boolean {
  return position(fen)?.isCheck() ?? false;
}

export interface Played {
  fen: string;
  san: string;
  uci: string;
}

/** Play a board move (orig, dest); pawns reaching the last rank become queens. */
export function playMove(fen: string, orig: string, dest: string): Played | null {
  const pos = position(fen);
  if (!pos) return null;
  const from = parseSquare(orig);
  const to = parseSquare(dest);
  if (from === undefined || to === undefined) return null;
  const piece = pos.board.get(from);
  const rank = Math.floor(to / 8);
  const promotion = piece?.role === 'pawn' && (rank === 0 || rank === 7) ? ('queen' as const) : undefined;
  const move = { from, to, promotion };
  if (!pos.isLegal(move)) return null;
  const san = makeSan(pos, move);
  pos.play(move);
  return { fen: makeFen(pos.toSetup()), san, uci: makeUci(move) };
}

export function playSan(fen: string, san: string): Played | null {
  const pos = position(fen);
  if (!pos) return null;
  const move = parseSan(pos, san.replace(/[!?]+$/, ''));
  if (!move) return null;
  const clean = makeSan(pos, move);
  const uci = makeUci(move);
  pos.play(move);
  return { fen: makeFen(pos.toSetup()), san: clean, uci };
}

export function playUci(fen: string, uci: string): Played | null {
  const pos = position(fen);
  const move = parseUci(uci);
  if (!pos || !move || !pos.isLegal(move)) return null;
  const san = makeSan(pos, move);
  pos.play(move);
  return { fen: makeFen(pos.toSetup()), san, uci };
}

export function uciSquares(uci: string): [Key, Key] | null {
  if (uci.length < 4) return null;
  return [uci.slice(0, 2) as Key, uci.slice(2, 4) as Key];
}

/** "12." / "12..." prefix for a move played from `fen`. */
export function moveNumber(fen: string): string {
  const parts = fen.split(' ');
  const n = parts[5] ?? '1';
  return parts[1] === 'b' ? `${n}...` : `${n}.`;
}

/** Numbered SAN line starting from `fen`. */
export function numberedLine(fen: string, sans: string[]): string {
  const parts = fen.split(' ');
  let n = parseInt(parts[5] ?? '1', 10);
  let white = parts[1] !== 'b';
  return sans
    .map((s, i) => {
      let out = '';
      if (white) out = `${n}. ${s}`;
      else out = i === 0 ? `${n}... ${s}` : s;
      if (!white) n += 1;
      white = !white;
      return out;
    })
    .join(' ');
}

// ---------------------------------------------------------------- scores

const MULT = -0.00368208;

export function whiteWin(score: Score): number {
  let cp: number;
  if (score.kind === 'mate') cp = score.value >= 0 ? 1000 : -1000;
  else cp = Math.max(-1000, Math.min(1000, score.value));
  return 50 + 50 * (2 / (1 + Math.exp(MULT * cp)) - 1);
}

export function fmtScore(score: Score | null | undefined): string {
  if (!score) return '';
  if (score.kind === 'mate') return score.value > 0 ? `#${score.value}` : `#-${-score.value}`;
  const v = score.value / 100;
  return (v > 0 ? '+' : '') + v.toFixed(v >= 10 || v <= -10 ? 0 : 1);
}

export const CLASS_LABEL: Record<Classification, string> = {
  book: 'Book',
  best: 'Best',
  excellent: 'Excellent',
  good: 'Good',
  inaccuracy: 'Inaccuracy',
  mistake: 'Mistake',
  blunder: 'Blunder',
  missed_win: 'Missed win'
};

export const CLASS_GLYPH: Record<Classification, string> = {
  book: '',
  best: '!!',
  excellent: '!',
  good: '',
  inaccuracy: '?!',
  mistake: '?',
  blunder: '??',
  missed_win: '⌀'
};

export function isError(c: Classification): boolean {
  return c === 'inaccuracy' || c === 'mistake' || c === 'blunder' || c === 'missed_win';
}

export function tagLabel(tag: string): string {
  const s = tag.replace(/_/g, ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
}
