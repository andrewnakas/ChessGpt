// PGN parsing, game phase and the opening book for browser mode.
import { Chess } from 'chessops/chess';
import { makeFen, parseFen } from 'chessops/fen';
import { isMate, parseComment, parsePgn, startingPosition } from 'chessops/pgn';
import { makeSan, parseSan } from 'chessops/san';
import { standardUci } from '../chess';
import type { Phase, PlyMove, Score } from '../api/types';

export const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';

export interface ParsedGame {
  tags: Map<string, string>;
  startFen: string;
  moves: PlyMove[];
}

function evalScore(ev: ReturnType<typeof parseComment>['evaluation']): Score | null {
  if (!ev) return null;
  if (isMate(ev)) return { kind: 'mate', value: ev.mate };
  return { kind: 'cp', value: Math.round(ev.pawns * 100) };
}

const NAG_JUDGEMENTS = new Set([1, 2, 3, 4, 5, 6]);

export function parseGames(pgn: string): ParsedGame[] {
  return parsePgn(pgn).map((g) => {
    const start = startingPosition(g.headers);
    if (start.isErr) throw new Error(`bad starting position: ${start.error.message}`);
    const pos = start.value as Chess;
    const startFen = makeFen(pos.toSetup());
    const moves: PlyMove[] = [];
    let ply = 0;
    for (const node of g.moves.mainline()) {
      const move = parseSan(pos, node.san);
      if (!move) throw new Error(`illegal move ${node.san} at ply ${ply + 1}`);
      const fenBefore = makeFen(pos.toSetup());
      const mover = pos.turn;
      const san = makeSan(pos, move);
      const uci = standardUci(pos, move);
      pos.play(move);
      ply += 1;
      let clock: number | null = null;
      let pgnEval: Score | null = null;
      for (const c of node.comments ?? []) {
        const pc = parseComment(c);
        if (pc.clock !== undefined) clock = Math.round(pc.clock * 1000);
        pgnEval = evalScore(pc.evaluation) ?? pgnEval;
      }
      const nag = (node.nags ?? []).find((n) => NAG_JUDGEMENTS.has(n)) ?? null;
      moves.push({
        ply,
        mover,
        san,
        uci,
        fen_before: fenBefore,
        fen_after: makeFen(pos.toSetup()),
        clock_ms: clock,
        pgn_eval: pgnEval,
        nag
      });
    }
    return { tags: g.headers, startFen, moves };
  });
}

export function tag(g: ParsedGame, name: string): string | null {
  const v = g.tags.get(name);
  return v && v !== '?' ? v : null;
}

export function position(fen: string): Chess {
  const s = parseFen(fen);
  if (s.isErr) throw new Error('invalid FEN');
  const p = Chess.fromSetup(s.value);
  if (p.isErr) throw new Error('illegal position');
  return p.value;
}

/** FEN without move counters (and en passant only when capturable, as chessops writes it). */
export function fenKey(fen: string): string {
  return fen.split(' ').slice(0, 4).join(' ');
}

/** Same approximation as chess-core position::phase. */
export function phase(fen: string): Phase {
  const board = fen.split(' ')[0];
  const pieces = [...board].filter((c) => 'nbrqNBRQ'.includes(c)).length;
  if (pieces <= 6) return 'endgame';
  const ranks = board.split('/');
  const count = (r: string, white: boolean) => [...r].filter((c) => /[a-zA-Z]/.test(c) && (c === c.toUpperCase()) === white).length;
  const whiteBack = count(ranks[7], true);
  const blackBack = count(ranks[0], false);
  return pieces <= 10 || whiteBack < 4 || blackBack < 4 ? 'middlegame' : 'opening';
}

// ---------------------------------------------------------------- book

interface Book {
  positions: Set<string>;
  names: Map<string, { eco: string; name: string }>;
}
let bookPromise: Promise<Book> | null = null;

export function loadBook(fetcher: typeof fetch = fetch): Promise<Book> {
  bookPromise ??= (async () => {
    const positions = new Set<string>();
    const names = new Map<string, { eco: string; name: string }>();
    for (const f of ['a', 'b', 'c', 'd', 'e']) {
      const text = await (await fetcher(`/openings/${f}.tsv`)).text();
      for (const line of text.split('\n').slice(1)) {
        const [eco, name, pgn] = line.split('\t');
        if (!pgn) continue;
        const pos = Chess.default();
        let ok = true;
        for (const tok of pgn.trim().split(/\s+/)) {
          if (/^\d+\./.test(tok)) continue;
          const m = parseSan(pos, tok);
          if (!m) {
            ok = false;
            break;
          }
          pos.play(m);
          positions.add(fenKey(makeFen(pos.toSetup())));
        }
        if (ok) names.set(fenKey(makeFen(pos.toSetup())), { eco, name });
      }
    }
    return { positions, names };
  })();
  return bookPromise;
}

export async function bookInfo(moves: PlyMove[], fetcher?: typeof fetch) {
  const book = await loadBook(fetcher);
  let still = true;
  let opening: { eco: string; name: string } | null = null;
  const flags = moves.map((m) => {
    const k = fenKey(m.fen_after);
    still = still && book.positions.has(k);
    opening = book.names.get(k) ?? opening;
    return still;
  });
  return { flags, opening: opening as { eco: string; name: string } | null };
}
