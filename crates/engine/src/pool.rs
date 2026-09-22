//! A pool of Stockfish workers with a priority queue, cancellation, and an
//! analysis cache (in-memory LRU plus an optional persistent store).

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::Duration;

use chess_core::position::{fen_key, parse_fen, to_fen};
use lru::LruCache;
use shakmaty::{Chess, Position};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::process::{EngineProcess, ProcessOptions};
use crate::types::{Analysis, AnalysisRequest, EngineError, Limit, Terminal};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub path: PathBuf,
    pub workers: usize,
    pub threads: u32,
    pub hash_mb: u32,
    pub cache_entries: usize,
}

impl EngineConfig {
    pub fn with_defaults(path: PathBuf) -> EngineConfig {
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let threads = cpus.clamp(1, 4) as u32;
        EngineConfig {
            path,
            workers: (cpus / threads as usize).clamp(1, 4),
            threads,
            hash_mb: 256,
            cache_entries: 20_000,
        }
    }
}

/// Persistent analysis cache, implemented by the db crate.
#[async_trait::async_trait]
pub trait AnalysisStore: Send + Sync {
    async fn get(&self, fen_key: &str, multipv: u32, min_depth: u32) -> Option<Analysis>;
    async fn put(&self, fen_key: &str, analysis: &Analysis);
}

struct Job {
    req: AnalysisRequest,
    pos: Chess,
    cancel: CancellationToken,
    updates: Option<mpsc::UnboundedSender<Analysis>>,
    done: oneshot::Sender<Result<Analysis, EngineError>>,
}

struct Queued {
    seq: u64,
    job: Job,
}

impl PartialEq for Queued {
    fn eq(&self, o: &Self) -> bool {
        self.seq == o.seq
    }
}
impl Eq for Queued {}
impl PartialOrd for Queued {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Queued {
    // Higher priority first, then FIFO.
    fn cmp(&self, o: &Self) -> Ordering {
        self.job.req.priority.cmp(&o.job.req.priority).then(o.seq.cmp(&self.seq))
    }
}

struct Inner {
    cfg: EngineConfig,
    queue: Mutex<BinaryHeap<Queued>>,
    notify: Notify,
    cache: Mutex<LruCache<(String, u32), (Limit, Analysis)>>,
    store: Option<Arc<dyn AnalysisStore>>,
    seq: AtomicU64,
    shutdown: CancellationToken,
    engine_name: String,
}

#[derive(Clone)]
pub struct EnginePool {
    inner: Arc<Inner>,
}

/// A running analysis: partial snapshots arrive on `updates`, the final
/// result on `result`. Dropping the handle cancels the search.
pub struct AnalysisHandle {
    pub updates: mpsc::UnboundedReceiver<Analysis>,
    pub result: oneshot::Receiver<Result<Analysis, EngineError>>,
    _guard: DropGuard,
}

impl EnginePool {
    pub async fn start(cfg: EngineConfig, store: Option<Arc<dyn AnalysisStore>>) -> Result<EnginePool, EngineError> {
        let opts = ProcessOptions { threads: cfg.threads, hash_mb: cfg.hash_mb };
        let first = EngineProcess::spawn(&cfg.path, &opts).await?;
        let engine_name = first.name.clone();
        tracing::info!(engine = %engine_name, workers = cfg.workers, threads = cfg.threads, "engine pool started");
        let inner = Arc::new(Inner {
            cache: Mutex::new(LruCache::new(NonZeroUsize::new(cfg.cache_entries.max(1)).unwrap())),
            cfg,
            queue: Mutex::new(BinaryHeap::new()),
            notify: Notify::new(),
            store,
            seq: AtomicU64::new(0),
            shutdown: CancellationToken::new(),
            engine_name,
        });
        let mut first = Some(first);
        for id in 0..inner.cfg.workers.max(1) {
            tokio::spawn(worker(inner.clone(), id, first.take()));
        }
        Ok(EnginePool { inner })
    }

    pub fn engine_name(&self) -> &str {
        &self.inner.engine_name
    }

    pub fn config(&self) -> &EngineConfig {
        &self.inner.cfg
    }

    pub fn shutdown(&self) {
        self.inner.shutdown.cancel();
        self.inner.notify.notify_waiters();
    }

    /// Analyse and wait for the final result.
    pub async fn analyse(&self, req: AnalysisRequest) -> Result<Analysis, EngineError> {
        let h = self.submit(req, false).await?;
        let AnalysisHandle { result, _guard, .. } = h;
        let r = result.await.map_err(|_| EngineError::Shutdown)?;
        drop(_guard);
        r
    }

    /// Analyse with streamed partial results.
    pub async fn analyse_stream(&self, req: AnalysisRequest) -> Result<AnalysisHandle, EngineError> {
        self.submit(req, true).await
    }

    async fn submit(&self, mut req: AnalysisRequest, stream: bool) -> Result<AnalysisHandle, EngineError> {
        if self.inner.shutdown.is_cancelled() {
            return Err(EngineError::Shutdown);
        }
        let pos = parse_fen(&req.fen).map_err(|e| EngineError::BadFen(e.to_string()))?;
        req.fen = to_fen(&pos);
        let cancel = CancellationToken::new();
        let (utx, urx) = mpsc::unbounded_channel();
        let (dtx, drx) = oneshot::channel();
        let guard = cancel.clone().drop_guard();

        if let Some(done) = terminal_analysis(&pos, &req, &self.inner.engine_name) {
            let _ = dtx.send(Ok(done));
            return Ok(AnalysisHandle { updates: urx, result: drx, _guard: guard });
        }
        if let Some(hit) = self.cached(&pos, &req).await {
            let _ = dtx.send(Ok(hit));
            return Ok(AnalysisHandle { updates: urx, result: drx, _guard: guard });
        }

        let job = Job { req, pos, cancel, updates: stream.then_some(utx), done: dtx };
        let seq = self.inner.seq.fetch_add(1, AtomicOrdering::Relaxed);
        self.inner.queue.lock().await.push(Queued { seq, job });
        self.inner.notify.notify_one();
        Ok(AnalysisHandle { updates: urx, result: drx, _guard: guard })
    }

    async fn cached(&self, pos: &Chess, req: &AnalysisRequest) -> Option<Analysis> {
        let key = fen_key(pos);
        let wanted_multipv = req.multipv.min(pos.legal_moves().len() as u32).max(1);
        {
            let mut cache = self.inner.cache.lock().await;
            for mpv in wanted_multipv..=10 {
                if let Some((limit, a)) = cache.get(&(key.clone(), mpv))
                    && satisfies(limit, a, &req.limit)
                {
                    return Some(with_fen(a.clone(), &req.fen, wanted_multipv));
                }
            }
        }
        let min_depth = req.limit.depth?;
        let store = self.inner.store.as_ref()?;
        let a = store.get(&key, wanted_multipv, min_depth).await?;
        self.inner
            .cache
            .lock()
            .await
            .put((key, a.multipv), (Limit::depth(a.depth), a.clone()));
        Some(with_fen(a, &req.fen, wanted_multipv))
    }
}

fn satisfies(cached_limit: &Limit, a: &Analysis, want: &Limit) -> bool {
    cached_limit == want || want.depth.is_some_and(|d| a.depth >= d)
}

fn with_fen(mut a: Analysis, fen: &str, multipv: u32) -> Analysis {
    a.fen = fen.to_string();
    a.lines.truncate(multipv as usize);
    a
}

fn terminal_analysis(pos: &Chess, req: &AnalysisRequest, engine: &str) -> Option<Analysis> {
    let terminal = if pos.is_checkmate() {
        Terminal::Checkmate
    } else if pos.is_stalemate() {
        Terminal::Stalemate
    } else {
        return None;
    };
    Some(Analysis {
        fen: req.fen.clone(),
        depth: 0,
        multipv: req.multipv,
        lines: vec![],
        nodes: 0,
        nps: 0,
        time_ms: 0,
        best_move: None,
        terminal: Some(terminal),
        done: true,
        engine: engine.to_string(),
    })
}

async fn next_job(inner: &Inner) -> Option<Job> {
    loop {
        if inner.shutdown.is_cancelled() {
            return None;
        }
        if let Some(q) = inner.queue.lock().await.pop() {
            return Some(q.job);
        }
        tokio::select! {
            _ = inner.notify.notified() => {}
            _ = inner.shutdown.cancelled() => return None,
        }
    }
}

async fn worker(inner: Arc<Inner>, id: usize, mut proc: Option<EngineProcess>) {
    let opts = ProcessOptions { threads: inner.cfg.threads, hash_mb: inner.cfg.hash_mb };
    while let Some(job) = next_job(&inner).await {
        if job.cancel.is_cancelled() {
            let _ = job.done.send(Err(EngineError::Cancelled));
            continue;
        }
        if proc.is_none() {
            match EngineProcess::spawn(&inner.cfg.path, &opts).await {
                Ok(p) => proc = Some(p),
                Err(e) => {
                    tracing::error!(worker = id, "engine spawn failed: {e}");
                    let _ = job.done.send(Err(e));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            }
        }
        let p = proc.as_mut().expect("engine present");
        let updates = job.updates.clone();
        let res = p
            .analyse(&job.req, job.pos.turn(), &job.cancel, |snap| {
                if let Some(tx) = &updates {
                    let _ = tx.send(snap.clone());
                }
            })
            .await;
        match &res {
            Ok(a) => {
                let key = (fen_key(&job.pos), a.multipv);
                inner.cache.lock().await.put(key.clone(), (job.req.limit, a.clone()));
                if let Some(store) = &inner.store {
                    store.put(&key.0, a).await;
                }
            }
            Err(EngineError::Cancelled) => {}
            Err(e) => {
                tracing::warn!(worker = id, "engine error, restarting: {e}");
                proc = None;
            }
        }
        let _ = job.done.send(res);
    }
    if let Some(p) = proc {
        p.quit().await;
    }
}
