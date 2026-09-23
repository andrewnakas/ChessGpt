// Browser mode: when the chessgpt server can't be reached, the same pages run
// on this backend. Stockfish runs in the browser, games live in IndexedDB.
// The coach, accounts and the Claude/ChatGPT connector need the server.
import type {
  AnalyseGameRequest,
  AnalyseGameResponse,
  EloTier,
  GameAnalysis,
  GameDetail,
  GameSummary,
  ImportRequest,
  ImportResponse,
  JobEvent,
  Meta,
  MoveEval,
  Score,
  Settings,
  SettingsInput,
  Side
} from '../api/types';
import { browserEngine } from './engine';
import { bookInfo, parseGames, phase, tag, type ParsedGame } from './game';
import { gameAccuracy, judge, keyMoments, moveAccuracy } from './judge';
import { progressFrom } from './progress';
import { baseSeconds, estimate } from './rating';
import { loadSettings, saveSettings, store, type StoredGame } from './store';

export class OfflineError extends Error {
  status = 503;
}

export const needsServer = (what: string) =>
  new OfflineError(`${what} needs the chessgpt server, which is offline right now. Analysis still works in your browser.`);

export const offlineMeta: Meta = {
  version: 'browser',
  engine: 'Stockfish 19 (in your browser)',
  engine_threads: 1,
  engine_workers: 1,
  mode: 'browser',
  has_provider: false,
  managed_provider: false,
  device_model: null,
  provider_label: null,
  budget_used: null
};

const uid = () => (crypto.randomUUID ? crypto.randomUUID() : String(Date.now()) + Math.random().toString(16).slice(2));

function tier(elo: number): EloTier {
  return elo < 1200 ? 'beginner' : elo < 1800 ? 'intermediate' : elo < 2200 ? 'advanced' : 'expert';
}

/** Browser engine depth per tier: the WebAssembly engine is slower than the server's. */
const DEPTH: Record<EloTier, number> = { beginner: 11, intermediate: 12, advanced: 13, expert: 14 };

async function hashKey(g: ParsedGame): Promise<string> {
  const text = g.startFen + g.moves.map((m) => m.uci).join('') + ['White', 'Black', 'Date', 'Result'].map((t) => tag(g, t) ?? '').join('|');
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(d)].slice(0, 12).map((b) => b.toString(16).padStart(2, '0')).join('');
}

async function saveParsed(pgn: string, g: ParsedGame, source: GameSummary['source'], sourceId: string | null, out: ImportResponse) {
  const sid = sourceId ?? (await hashKey(g));
  const existing = (await store.all()).find((x) => x.summary.source === source && x.summary.source_id === sid);
  if (existing) {
    out.duplicates += 1;
    out.games.push(existing.summary);
    return;
  }
  const settings = loadSettings();
  const { opening } = await bookInfo(g.moves);
  const white = tag(g, 'White') ?? '?';
  const black = tag(g, 'Black') ?? '?';
  const me = [settings.lichess_username, settings.chesscom_username].filter(Boolean).map((u) => u!.toLowerCase());
  const userSide: Side | null = me.includes(white.toLowerCase()) ? 'white' : me.includes(black.toLowerCase()) ? 'black' : null;
  const elo = (t: string) => {
    const v = parseInt(tag(g, t) ?? '', 10);
    return Number.isFinite(v) ? v : null;
  };
  const summary: GameSummary = {
    id: uid(),
    source,
    source_id: sid,
    white,
    black,
    white_elo: elo('WhiteElo'),
    black_elo: elo('BlackElo'),
    result: tag(g, 'Result') ?? '*',
    date: tag(g, 'UTCDate') ?? tag(g, 'Date'),
    time_control: tag(g, 'TimeControl'),
    eco: opening?.eco ?? tag(g, 'ECO'),
    opening: opening?.name ?? tag(g, 'Opening'),
    user_side: userSide,
    ply_count: g.moves.length,
    analysis_status: null,
    white_accuracy: null,
    black_accuracy: null,
    imported_at: Date.now()
  };
  await store.put({ summary, pgn, start_fen: g.startFen, moves: g.moves, analysis: null });
  out.games.push(summary);
}

function splitPgns(text: string): string[] {
  return text
    .replace(/\r\n/g, '\n')
    .split(/\n\s*\n(?=\[)/)
    .map((s) => s.trim())
    .filter(Boolean);
}

async function importGames(r: ImportRequest): Promise<ImportResponse> {
  const out: ImportResponse = { games: [], duplicates: 0, errors: [] };
  const addText = async (text: string, source: GameSummary['source'], idOf?: (pgn: string) => string | null) => {
    for (const chunk of splitPgns(text)) {
      try {
        for (const g of parseGames(chunk)) await saveParsed(chunk, g, source, idOf?.(chunk) ?? null, out);
      } catch (e) {
        out.errors.push((e as Error).message);
      }
    }
  };
  if (r.source === 'pgn') await addText(r.pgn, 'pgn');
  else if (r.source === 'fen') throw needsServer('Saving a single position');
  else if (r.source === 'lichess_game') {
    const id = (r.id.match(/lichess\.org\/(?:game\/export\/)?([A-Za-z0-9]{8})/)?.[1] ?? r.id.trim()).slice(0, 8);
    const res = await fetch(`https://lichess.org/game/export/${id}?clocks=true&evals=false`, {
      headers: { accept: 'application/x-chess-pgn' }
    });
    if (!res.ok) throw new Error(`Lichess returned ${res.status} for game ${id}`);
    await addText(await res.text(), 'lichess', () => id);
  } else if (r.source === 'lichess') {
    throw needsServer('Importing a Lichess user’s games');
  } else if (r.source === 'chesscom') {
    const user = r.username.trim().toLowerCase();
    const max = Math.max(1, Math.min(100, r.max ?? 20));
    const arch = await fetch(`https://api.chess.com/pub/player/${user}/games/archives`);
    if (!arch.ok) throw new Error(arch.status === 404 ? `Chess.com user ${user} not found` : `Chess.com returned ${arch.status}`);
    const months: string[] = (await arch.json()).archives ?? [];
    let n = 0;
    for (const m of months.reverse()) {
      if (n >= max) break;
      const games = ((await (await fetch(m)).json()).games ?? []) as { url: string; pgn?: string; rules?: string; end_time?: number }[];
      for (const g of games.filter((g) => (g.rules ?? 'chess') === 'chess' && g.pgn).sort((a, b) => (b.end_time ?? 0) - (a.end_time ?? 0))) {
        if (n >= max) break;
        await addText(g.pgn!, 'chesscom', () => g.url);
        n++;
      }
    }
  }
  if (!out.games.length && out.errors.length) throw new Error(out.errors.join('; '));
  return out;
}

// ---------------------------------------------------------------- jobs

interface LiveJob {
  events: JobEvent[];
  listeners: Set<(e: JobEvent) => void>;
  cancelled: boolean;
}
const jobs = new Map<string, LiveJob>();

function emit(id: string, e: JobEvent) {
  const j = jobs.get(id);
  if (!j) return;
  j.events.push(e);
  j.listeners.forEach((f) => f(e));
}

async function runAnalysis(gameId: string, analysis: GameAnalysis) {
  const id = analysis.id;
  const g = (await store.get(gameId))!;
  const depth = DEPTH[analysis.tier];
  const fens = [g.start_fen, ...g.moves.map((m) => m.fen_after)];
  const { flags } = await bookInfo(g.moves);
  const engine = browserEngine();
  const scores: Score[] = [];
  const results: { score: Score; best: string | null; line: string[] }[] = [];
  for (let i = 0; i < fens.length; i++) {
    if (jobs.get(id)?.cancelled) throw new Error('cancelled');
    const a = await engine.analyse(fens[i], { depth });
    const white = fens[i].split(' ')[1] !== 'b';
    const score: Score =
      a.terminal === 'checkmate' ? { kind: 'mate', value: white ? -1 : 1 } : a.terminal === 'stalemate' ? { kind: 'cp', value: 0 } : (a.lines[0]?.score ?? { kind: 'cp', value: 0 });
    results.push({ score, best: a.lines[0]?.pv_uci[0] ?? null, line: a.lines[0]?.pv_san.slice(0, 10) ?? [] });
    if (i === 0) {
      analysis.start_score = score;
      continue;
    }
    const pm = g.moves[i - 1];
    const prev = results[i - 1];
    const v = judge({ mover: pm.mover, prev: prev.score, cur: score, bestUci: prev.best, playedUci: pm.uci, isBook: flags[i - 1] });
    const m: MoveEval = {
      ply: pm.ply,
      mover: pm.mover,
      san: pm.san,
      uci: pm.uci,
      score,
      depth: a.depth,
      best_uci: prev.best,
      best_san: prev.line[0] ?? null,
      best_line_san: prev.line,
      classification: v.classification,
      lichess_judgement: v.lichess,
      win_before: v.winBefore,
      win_after: v.winAfter,
      delta_wc: v.deltaWc,
      accuracy: moveAccuracy(v.winBefore, v.winAfter),
      phase: phase(pm.fen_before),
      is_key_moment: false
    };
    analysis.moves.push(m);
    scores.push(score);
    emit(id, { type: 'move', eval: m });
    emit(id, { type: 'progress', stage: 'engine', done: i, total: g.moves.length });
  }
  const acc = gameAccuracy(g.moves[0]?.mover !== 'black', scores);
  analysis.white_accuracy = acc.white;
  analysis.black_accuracy = acc.black;
  emit(id, { type: 'accuracy', white: acc.white, black: acc.black });
  const base = baseSeconds(g.summary.time_control);
  analysis.white_estimate = estimate(analysis.moves.filter((m) => m.mover === 'white'), base);
  analysis.black_estimate = estimate(analysis.moves.filter((m) => m.mover === 'black'), base);
  const keys = keyMoments(analysis.moves, analysis.user_side, g.summary.result);
  analysis.key_moments = keys;
  analysis.moves = analysis.moves.map((m) => ({ ...m, is_key_moment: keys.includes(m.ply) }));
  emit(id, { type: 'key_moments', plies: keys });
  analysis.status = 'done';
}

async function startAnalysis(gameId: string, req: AnalyseGameRequest): Promise<AnalyseGameResponse> {
  const g = await store.get(gameId);
  if (!g) throw new Error('game not found');
  if (!g.moves.length) throw new Error('this game has no moves');
  if (!req.force && g.analysis && g.analysis.status !== 'failed' && g.analysis.status !== 'cancelled' && (g.analysis.status === 'done' || jobs.has(g.analysis.id)))
    return { analysis_id: g.analysis.id, status: g.analysis.status };
  const elo = req.elo ?? loadSettings().elo;
  const analysis: GameAnalysis = {
    id: uid(),
    game_id: gameId,
    status: 'running',
    elo,
    tier: tier(elo),
    user_side: req.user_side ?? g.summary.user_side,
    engine: offlineMeta.engine,
    start_score: null,
    white_accuracy: null,
    black_accuracy: null,
    white_estimate: null,
    black_estimate: null,
    moves: [],
    key_moments: [],
    explanations: [],
    review: null,
    error: null,
    created_at: Date.now()
  };
  g.analysis = analysis;
  g.summary.user_side = analysis.user_side;
  g.summary.analysis_status = 'running';
  await store.put(g);
  jobs.set(analysis.id, { events: [], listeners: new Set(), cancelled: false });
  runAnalysis(gameId, analysis)
    .catch((e) => {
      analysis.status = jobs.get(analysis.id)?.cancelled ? 'cancelled' : 'failed';
      analysis.error = (e as Error).message;
    })
    .finally(async () => {
      const fresh = (await store.get(gameId))!;
      fresh.analysis = analysis;
      fresh.summary.analysis_status = analysis.status;
      fresh.summary.white_accuracy = analysis.white_accuracy;
      fresh.summary.black_accuracy = analysis.black_accuracy;
      await store.put(fresh);
      emit(analysis.id, { type: 'done', analysis });
      jobs.delete(analysis.id);
    });
  return { analysis_id: analysis.id, status: 'running' };
}

async function findAnalysis(id: string): Promise<StoredGame> {
  const g = (await store.all()).find((x) => x.analysis?.id === id);
  if (!g) throw new Error('analysis not found');
  return g;
}

export const offline = {
  meta: async () => offlineMeta,
  games: async () => (await store.all()).map((g) => g.summary).sort((a, b) => b.imported_at - a.imported_at),
  game: async (id: string): Promise<GameDetail> => {
    const g = await store.get(id);
    if (!g) throw new Error('game not found');
    return { summary: g.summary, pgn: g.pgn, start_fen: g.start_fen, moves: g.moves, analysis: g.analysis };
  },
  deleteGame: async (id: string) => {
    await store.delete(id);
  },
  setSide: async (id: string, side: Side | null) => {
    const g = (await store.get(id))!;
    g.summary.user_side = side;
    await store.put(g);
    return g.summary;
  },
  importGames: importGames,
  analyse: startAnalysis,
  analysis: async (id: string) => (await findAnalysis(id)).analysis!,
  progress: async () => progressFrom(await store.all()),
  cancelAnalysis: async (id: string) => {
    const j = jobs.get(id);
    if (j) j.cancelled = true;
  },
  settings: async (): Promise<Settings> => loadSettings(),
  saveSettings: async (s: SettingsInput): Promise<Settings> => {
    const v: Settings = {
      elo: Math.max(100, Math.min(3500, s.elo)),
      lichess_username: s.lichess_username?.trim() || null,
      chesscom_username: s.chesscom_username?.trim() || null,
      explorer_enabled: false,
      has_lichess_token: false
    };
    saveSettings(v);
    return v;
  },
  /** Replays events so far, then streams live ones until `done`. */
  jobEvents: async (id: string, on: (e: JobEvent) => void, signal: AbortSignal) => {
    const g = await findAnalysis(id);
    on({ type: 'snapshot', analysis: g.analysis! });
    const j = jobs.get(id);
    if (!j) {
      on({ type: 'done', analysis: g.analysis! });
      return;
    }
    await new Promise<void>((resolve) => {
      const f = (e: JobEvent) => {
        on(e);
        if (e.type === 'done') {
          j.listeners.delete(f);
          resolve();
        }
      };
      j.listeners.add(f);
      signal.addEventListener('abort', () => {
        j.listeners.delete(f);
        resolve();
      });
    });
  }
};
