// Stockfish 19 (lite, single-threaded WebAssembly) in a Web Worker. Output
// uses the same shapes as the server's engine API.
import type { EngineAnalysis, LineDto, Score } from '../api/types';
import { playUci, position as pos } from '../chess';

const ENGINE_URL = '/engine/stockfish-19-lite-single.js';

interface Job {
  fen: string;
  multipv: number;
  depth: number;
  movetimeMs?: number;
  onUpdate?: (a: EngineAnalysis) => void;
  resolve: (a: EngineAnalysis) => void;
  reject: (e: Error) => void;
  signal?: AbortSignal;
}

function toSan(fen: string, pv: string[]): string[] {
  const out: string[] = [];
  let f = fen;
  for (const u of pv) {
    const r = playUci(f, u);
    if (!r) break;
    out.push(r.san);
    f = r.fen;
  }
  return out;
}

/** The subset of Worker the engine needs (a Node child process in tests). */
export interface EngineWorker {
  postMessage(cmd: string): void;
  onmessage: ((e: { data: unknown }) => void) | null;
  onerror: ((e: { message: string }) => void) | null;
}

let makeWorker: () => EngineWorker = () => new Worker(ENGINE_URL) as unknown as EngineWorker;
/** Tests swap in a different engine transport. */
export function setEngineWorkerFactory(f: () => EngineWorker) {
  makeWorker = f;
  instance = null;
}

export class BrowserEngine {
  private worker: EngineWorker | null = null;
  private ready: Promise<void> | null = null;
  private queue: Job[] = [];
  private current: Job | null = null;
  private lines = new Map<number, LineDto>();
  private meta = { depth: 0, nodes: 0, nps: 0, time: 0 };
  private listener: ((line: string) => void) | null = null;
  private lastEmit = 0;

  private start(): Promise<void> {
    this.ready ??= new Promise((resolve, reject) => {
      const w = makeWorker();
      this.worker = w;
      w.onmessage = (e) => {
        const line = String(e.data);
        if (this.listener) this.listener(line);
      };
      w.onerror = (e) => reject(new Error(`engine failed to load: ${e.message}`));
      this.listener = (line) => {
        if (line === 'uciok') w.postMessage('isready');
        if (line === 'readyok') {
          this.listener = (l) => this.onLine(l);
          resolve();
        }
      };
      w.postMessage('uci');
      w.postMessage('setoption name UCI_ShowWDL value true');
      w.postMessage('setoption name Hash value 32');
    });
    return this.ready;
  }

  analyse(
    fen: string,
    opts: { multipv?: number; depth?: number; movetimeMs?: number; onUpdate?: (a: EngineAnalysis) => void; signal?: AbortSignal } = {}
  ): Promise<EngineAnalysis> {
    return new Promise((resolve, reject) => {
      const job: Job = {
        fen,
        multipv: Math.max(1, Math.min(5, opts.multipv ?? 1)),
        depth: opts.depth ?? 16,
        movetimeMs: opts.movetimeMs,
        onUpdate: opts.onUpdate,
        resolve,
        reject,
        signal: opts.signal
      };
      opts.signal?.addEventListener('abort', () => {
        if (this.current === job) this.worker?.postMessage('stop');
        else {
          this.queue = this.queue.filter((j) => j !== job);
          reject(new DOMException('aborted', 'AbortError'));
        }
      });
      this.queue.push(job);
      this.pump();
    });
  }

  private async pump() {
    if (this.current || !this.queue.length) return;
    await this.start();
    if (this.current) return;
    const job = this.queue.shift()!;
    this.current = job;
    this.lines.clear();
    this.meta = { depth: 0, nodes: 0, nps: 0, time: 0 };
    const p = pos(job.fen);
    if (p && (p.isCheckmate() || p.isStalemate())) {
      this.current = null;
      job.resolve(this.snapshot(job, true, p.isCheckmate() ? 'checkmate' : 'stalemate'));
      this.pump();
      return;
    }
    const w = this.worker!;
    w.postMessage(`setoption name MultiPV value ${job.multipv}`);
    w.postMessage(`position fen ${job.fen}`);
    w.postMessage(`go depth ${job.depth}${job.movetimeMs ? ` movetime ${job.movetimeMs}` : ''}`);
  }

  private snapshot(job: Job, done: boolean, terminal: 'checkmate' | 'stalemate' | null = null): EngineAnalysis {
    return {
      fen: job.fen,
      depth: this.meta.depth,
      lines: [...this.lines.values()].sort((a, b) => a.rank - b.rank),
      nodes: this.meta.nodes,
      nps: this.meta.nps,
      time_ms: this.meta.time,
      terminal,
      done,
      engine: 'Stockfish 19 (in your browser)'
    };
  }

  private onLine(line: string) {
    const job = this.current;
    if (!job) return;
    if (line.startsWith('bestmove')) {
      this.current = null;
      if (job.signal?.aborted) job.reject(new DOMException('aborted', 'AbortError'));
      else job.resolve(this.snapshot(job, true));
      this.pump();
      return;
    }
    if (!line.startsWith('info') || !line.includes(' pv ')) return;
    const t = line.split(' ');
    const num = (k: string) => {
      const i = t.indexOf(k);
      return i >= 0 ? Number(t[i + 1]) : undefined;
    };
    const si = t.indexOf('score');
    if (si < 0) return;
    if (t[si + 3] === 'lowerbound' || t[si + 3] === 'upperbound') return;
    const white = job.fen.split(' ')[1] !== 'b';
    const raw = Number(t[si + 2]);
    const v = white ? raw : -raw;
    const score: Score = t[si + 1] === 'mate' ? { kind: 'mate', value: v } : { kind: 'cp', value: v };
    const wi = t.indexOf('wdl');
    let wdl: [number, number, number] | null = null;
    if (wi >= 0) {
      const [a, b, c] = [Number(t[wi + 1]), Number(t[wi + 2]), Number(t[wi + 3])];
      wdl = white ? [a, b, c] : [c, b, a];
    }
    const pv = t.slice(t.indexOf('pv') + 1);
    const rank = num('multipv') ?? 1;
    const depth = num('depth') ?? 0;
    this.lines.set(rank, { rank, depth, score, wdl, pv_uci: pv, pv_san: toSan(job.fen, pv) });
    if (rank === 1) this.meta.depth = depth;
    this.meta.nodes = num('nodes') ?? this.meta.nodes;
    this.meta.nps = num('nps') ?? this.meta.nps;
    this.meta.time = num('time') ?? this.meta.time;
    const now = performance.now();
    if (job.onUpdate && now - this.lastEmit > 120) {
      this.lastEmit = now;
      job.onUpdate(this.snapshot(job, false));
    }
  }
}

let instance: BrowserEngine | null = null;
export function browserEngine(): BrowserEngine {
  instance ??= new BrowserEngine();
  return instance;
}
