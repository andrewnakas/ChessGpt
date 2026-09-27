// Typed API client. Types come from crates/api-types via `cargo xtask gen-types`.
import { browserEngine } from '../offline/engine';
import { OfflineError, needsServer, offline } from '../offline/backend';
import * as guestDrills from '../training/guestDrills';
import type {
  AnalyseGameRequest,
  AnalyseGameResponse,
  ApiError,
  ChatEvent,
  ChatThread,
  ChatThreadDetail,
  CreateThreadRequest,
  EngineEvent,
  Explanation,
  GameAnalysis,
  GameDetail,
  GameSummary,
  DrillAttempt,
  DrillOverview,
  DrillSet,
  DrillSource,
  MistakeMotif,
  Progress,
  Puzzle,
  PuzzleQueue,
  SyncReport,
  ImportRequest,
  ImportResponse,
  JobEvent,
  Meta,
  Provider,
  ProviderInput,
  ProviderTestResult,
  SendMessageRequest,
  Settings,
  SettingsInput,
  Side
} from './types';

export interface Account {
  id: string;
  display_name: string;
  email: string | null;
  lichess_username: string | null;
  kind: string;
}

export interface SessionInfo {
  accounts: boolean;
  account: Account | null;
  lichess_login: boolean;
}

export class HttpError extends Error {
  constructor(
    public status: number,
    message: string
  ) {
    super(message);
  }
}

/** Set by the layout to show the login screen on 401. */
export const authListeners = new Set<() => void>();

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body)
  });
  if (res.status === 401) authListeners.forEach((f) => f());
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      msg = ((await res.json()) as ApiError).error;
    } catch {
      /* not json */
    }
    throw new HttpError(res.status, msg);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

const serverApi = {
  meta: () => request<Meta>('GET', '/meta'),
  session: () => request<SessionInfo>('GET', '/session'),
  register: (email: string, password: string, name?: string) =>
    request<Account>('POST', '/auth/register', { email, password, name: name ?? null }),
  login: (email: string, password: string) => request<Account>('POST', '/auth/login', { email, password }),
  logout: () => request<void>('POST', '/auth/logout'),
  share: (gameId: string) => request<{ token: string; path: string }>('POST', `/games/${gameId}/share`),
  shared: (token: string) => request<GameDetail>('GET', `/share/${token}`),
  claim: (token: string) => request<{ game_id: string }>('POST', `/share/${token}/claim`),
  connections: () => request<{ client_id: string; name: string; last_used: number }[]>('GET', '/connections'),
  disconnect: (clientId: string) => request<void>('DELETE', `/connections/${encodeURIComponent(clientId)}`),

  games: (limit = 200) => request<GameSummary[]>('GET', `/games?limit=${limit}`),
  game: (id: string) => request<GameDetail>('GET', `/games/${id}`),
  deleteGame: (id: string) => request<void>('DELETE', `/games/${id}`),
  setSide: (id: string, side: Side | null) => request<GameSummary>('PUT', `/games/${id}/side`, { side }),
  importGames: (r: ImportRequest) => request<ImportResponse>('POST', '/games/import', r),
  analyse: (id: string, r: AnalyseGameRequest) => request<AnalyseGameResponse>('POST', `/games/${id}/analyse`, r),
  analysis: (id: string) => request<GameAnalysis>('GET', `/analyses/${id}`),
  cancelAnalysis: (id: string) => request<void>('POST', `/analyses/${id}/cancel`),
  explainPly: (id: string, ply: number) => request<Explanation>('POST', `/analyses/${id}/explain/${ply}`),
  progress: () => request<Progress>('GET', '/progress'),
  connectChesscom: (username: string) => request<SyncReport>('POST', '/accounts/chesscom', { username }),
  disconnectChesscom: () => request<void>('DELETE', '/accounts/chesscom'),
  syncAccounts: () => request<SyncReport>('POST', '/accounts/sync'),
  puzzles: () => request<PuzzleQueue>('GET', '/puzzles'),
  puzzleAttempt: (id: string, solved: boolean) => request<Puzzle>('POST', `/puzzles/${id}/attempt`, { solved }),
  analysisMotifs: (id: string) => request<MistakeMotif[]>('GET', `/analyses/${id}/motifs`),
  drills: () => request<DrillOverview>('GET', '/drills'),
  createDrill: (source: DrillSource) => request<DrillSet>('POST', '/drills', source),
  drill: (id: string) => request<DrillSet>('GET', `/drills/${id}`),
  drillAttempt: (setId: string, itemId: string, a: DrillAttempt) =>
    request<DrillSet>('POST', `/drills/${setId}/items/${itemId}/attempt`, a),

  settings: () => request<Settings>('GET', '/settings'),
  saveSettings: (s: SettingsInput) => request<Settings>('PUT', '/settings', s),
  providers: () => request<Provider[]>('GET', '/providers'),
  createProvider: (p: ProviderInput) => request<Provider>('POST', '/providers', p),
  updateProvider: (id: string, p: ProviderInput) => request<Provider>('PUT', `/providers/${id}`, p),
  deleteProvider: (id: string) => request<void>('DELETE', `/providers/${id}`),
  testProvider: (id: string) => request<ProviderTestResult>('POST', `/providers/${id}/test`),

  threads: (gameId?: string) =>
    request<ChatThread[]>('GET', `/chat/threads${gameId ? `?game_id=${encodeURIComponent(gameId)}` : ''}`),
  createThread: (r: CreateThreadRequest) => request<ChatThread>('POST', '/chat/threads', r),
  thread: (id: string) => request<ChatThreadDetail>('GET', `/chat/threads/${id}`),
  deleteThread: (id: string) => request<void>('DELETE', `/chat/threads/${id}`)
};

/**
 * Read a server-sent-event stream with fetch (works for POST and supports
 * abort). Calls `onEvent` for every `data:` payload, parsed as JSON.
 */
export async function sse<T>(
  path: string,
  onEvent: (e: T) => void,
  opts: { method?: string; body?: unknown; signal?: AbortSignal } = {}
): Promise<void> {
  const res = await fetch(`/api${path}`, {
    method: opts.method ?? 'GET',
    headers: {
      accept: 'text/event-stream',
      ...(opts.body !== undefined ? { 'content-type': 'application/json' } : {})
    },
    body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
    signal: opts.signal
  });
  if (res.status === 401) authListeners.forEach((f) => f());
  if (!res.ok || !res.body) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      msg = ((await res.json()) as ApiError).error;
    } catch {
      /* ignore */
    }
    throw new HttpError(res.status, msg);
  }
  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
  let buf = '';
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += value;
    let idx: number;
    while ((idx = buf.search(/\r?\n\r?\n/)) >= 0) {
      const chunk = buf.slice(0, idx);
      buf = buf.slice(idx).replace(/^\r?\n\r?\n/, '');
      const data = chunk
        .split(/\r?\n/)
        .filter((l) => l.startsWith('data:'))
        .map((l) => l.slice(5).replace(/^ /, ''))
        .join('\n');
      if (data) onEvent(JSON.parse(data) as T);
    }
  }
}

const serverStreams = {
  engine: (fen: string, multipv: number, onEvent: (e: EngineEvent) => void, signal: AbortSignal, depth = 30) =>
    sse<EngineEvent>(
      `/engine/analyse?fen=${encodeURIComponent(fen)}&multipv=${multipv}&depth=${depth}&movetime_ms=30000`,
      onEvent,
      { signal }
    ),
  job: (analysisId: string, onEvent: (e: JobEvent) => void, signal: AbortSignal) =>
    sse<JobEvent>(`/analyses/${analysisId}/events`, onEvent, { signal }),
  chat: (threadId: string, body: SendMessageRequest, onEvent: (e: ChatEvent) => void, signal: AbortSignal) =>
    sse<ChatEvent>(`/chat/threads/${threadId}/messages`, onEvent, { method: 'POST', body, signal })
};

// ---------------------------------------------------------------- mode

export type Mode = 'server' | 'browser';
let modePromise: Promise<Mode> | null = null;

// Guests (hosted server, not signed in) use the browser backend too, like
// offline mode, but still reach the server for sign-in and guest drills.
let guest = false;
export function setGuest(on: boolean) {
  guest = on;
}
export const isGuest = () => guest;

/** Where calls go: the server, or this browser (offline, or a guest). */
export async function effectiveMode(): Promise<Mode> {
  return guest ? 'browser' : mode();
}

/** Calls that always go to the server when it is up. */
const SERVER_ALWAYS = new Set(['session', 'register', 'login', 'logout', 'shared', 'claim']);

/** Is the chessgpt server reachable? If not, everything runs in the browser. */
export function mode(): Promise<Mode> {
  modePromise ??= (async () => {
    if (typeof window === 'undefined') return 'server';
    try {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), 5000);
      const r = await fetch('/api/health', { signal: ctrl.signal, cache: 'no-store' });
      clearTimeout(t);
      return r.ok && (await r.text()).trim() === 'ok' ? 'server' : 'browser';
    } catch {
      return 'browser';
    }
  })();
  return modePromise;
}

type Fn = (...args: never[]) => Promise<unknown>;
function routed<S extends Record<string, Fn>>(server: S, local: Partial<{ [K in keyof S]: S[K] }>, what: Partial<Record<keyof S, string>>): S {
  const out: Record<string, Fn> = {};
  for (const k of Object.keys(server)) {
    out[k] = (async (...args: never[]) => {
      const m = await mode();
      if (m === 'server' && (!guest || SERVER_ALWAYS.has(k))) return server[k](...args);
      const f = local[k as keyof S];
      if (f) {
        try {
          return await f(...args);
        } catch (e) {
          // The server is up; a guest just has no account.
          if (guest && m === 'server' && e instanceof OfflineError)
            throw new Error(e.message.replace(/needs the chessgpt server, which is offline right now\./, 'needs a free account.'));
          throw e;
        }
      }
      const feature = what[k as keyof S] ?? 'This feature';
      if (guest && m === 'server') throw new Error(`${feature} needs a free account. Sign in or create one to use it.`);
      throw needsServer(feature);
    }) as Fn;
  }
  return out as S;
}

const browserSession: SessionInfo = { accounts: false, account: null, lichess_login: false };

export const api = routed(
  serverApi,
  {
    meta: offline.meta,
    session: async () => browserSession,
    games: offline.games,
    game: offline.game,
    deleteGame: offline.deleteGame,
    setSide: offline.setSide,
    importGames: offline.importGames,
    analyse: offline.analyse,
    analysis: offline.analysis,
    cancelAnalysis: offline.cancelAnalysis,
    progress: offline.progress,
    puzzles: async () => ({ due: [], total: 0, due_count: 0, learned: 0 }),
    analysisMotifs: async () => [],
    drills: async () =>
      guestDrills.overview(await guestWeakTags(), guestDrills.dailyStatus(await request<DrillSet>('GET', '/drills/daily'))),
    createDrill: async (source: DrillSource) => {
      if (source.kind === 'daily') return guestDrills.startDaily(await request<DrillSet>('GET', '/drills/daily'));
      const tag = guestDrills.guestTag(source, await guestWeakTags());
      const rating = (await offline.settings()).elo;
      return guestDrills.remember(await request<DrillSet>('POST', '/drills/try', { tag, rating }));
    },
    drill: async (id: string) => guestDrills.get(id),
    drillAttempt: async (setId: string, itemId: string, a: DrillAttempt) => guestDrills.attempt(setId, itemId, a),
    settings: offline.settings,
    saveSettings: offline.saveSettings,
    providers: async () => [],
    threads: async () => [],
    connections: async () => []
  } as Partial<typeof serverApi>,
  {
    explainPly: 'The coach',
    createThread: 'The coach',
    thread: 'The coach',
    share: 'Share links',
    shared: 'Shared games',
    claim: 'Saving shared games',
    register: 'Accounts',
    login: 'Accounts',
    createProvider: 'AI providers',
    testProvider: 'AI providers',
    connectChesscom: 'Linking a Chess.com account',
    syncAccounts: 'Syncing games',
    drills: 'Technique drills',
    createDrill: 'Technique drills',
    drill: 'Technique drills'
  }
);

async function guestWeakTags(): Promise<string[]> {
  try {
    return (await offline.progress()).motifs.map((m) => m.tag);
  } catch {
    return [];
  }
}

export const streams = {
  engine: async (fen: string, multipv: number, onEvent: (e: EngineEvent) => void, signal: AbortSignal, depth = 30) => {
    if ((await effectiveMode()) === 'server') return serverStreams.engine(fen, multipv, onEvent, signal, depth);
    const a = await browserEngine().analyse(fen, {
      multipv,
      depth: Math.min(depth, 22),
      movetimeMs: 20000,
      signal,
      onUpdate: (u) => onEvent({ type: 'update', analysis: u })
    });
    onEvent({ type: 'done', analysis: a });
  },
  job: async (analysisId: string, onEvent: (e: JobEvent) => void, signal: AbortSignal) => {
    if ((await effectiveMode()) === 'server') return serverStreams.job(analysisId, onEvent, signal);
    return offline.jobEvents(analysisId, onEvent, signal);
  },
  chat: async (threadId: string, body: SendMessageRequest, onEvent: (e: ChatEvent) => void, signal: AbortSignal) => {
    if ((await effectiveMode()) === 'server') return serverStreams.chat(threadId, body, onEvent, signal);
    throw needsServer('The coach');
  }
};
