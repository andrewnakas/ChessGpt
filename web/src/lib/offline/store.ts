// Browser-mode storage: games and analyses in IndexedDB, settings in localStorage.
import type { GameAnalysis, GameSummary, PlyMove, Settings } from '../api/types';

export interface StoredGame {
  summary: GameSummary;
  pgn: string;
  start_fen: string;
  moves: PlyMove[];
  analysis: GameAnalysis | null;
}

const DB = 'chessgpt-browser';
let dbp: Promise<IDBDatabase> | null = null;

function open(): Promise<IDBDatabase> {
  dbp ??= new Promise((resolve, reject) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore('games', { keyPath: 'summary.id' });
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  return dbp;
}

function tx<T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  return open().then(
    (db) =>
      new Promise<T>((resolve, reject) => {
        const req = f(db.transaction('games', mode).objectStore('games'));
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      })
  );
}

export const store = {
  all: () => tx<StoredGame[]>('readonly', (s) => s.getAll() as IDBRequest<StoredGame[]>),
  get: (id: string) => tx<StoredGame | undefined>('readonly', (s) => s.get(id) as IDBRequest<StoredGame | undefined>),
  put: (g: StoredGame) => tx('readwrite', (s) => s.put(g)),
  delete: (id: string) => tx('readwrite', (s) => s.delete(id))
};

const SETTINGS_KEY = 'chessgpt-settings';

export function loadSettings(): Settings {
  try {
    const v = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? 'null');
    if (v) return { ...v, has_lichess_token: false };
  } catch {
    /* storage blocked */
  }
  return { elo: 1500, lichess_username: null, chesscom_username: null, explorer_enabled: false, has_lichess_token: false };
}

export function saveSettings(s: Settings) {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(s));
  } catch {
    /* storage blocked */
  }
}
