//! One Stockfish child process speaking UCI over pipes.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use chess_core::Score;
use shakmaty::Color;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::types::{Analysis, AnalysisRequest, EngineError, PvLine};
use crate::uci::{Bound, UciLine, parse_line};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
const EMIT_INTERVAL: Duration = Duration::from_millis(120);

pub struct EngineProcess {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::UnboundedReceiver<String>,
    pub name: String,
    multipv: u32,
}

pub struct ProcessOptions {
    pub threads: u32,
    pub hash_mb: u32,
}

impl EngineProcess {
    pub async fn spawn(path: &Path, opts: &ProcessOptions) -> Result<EngineProcess, EngineError> {
        if !path.exists() {
            return Err(EngineError::NotFound(path.display().to_string()));
        }
        let mut cmd = Command::new(path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().map_err(|e| EngineError::Spawn(e.to_string()))?;
        let stdin = child.stdin.take().ok_or(EngineError::Spawn("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or(EngineError::Spawn("no stdout".into()))?;
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut p = EngineProcess { child, stdin, lines: rx, name: String::new(), multipv: 1 };
        p.send("uci").await?;
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            match parse_line(&p.recv_until(deadline).await?) {
                UciLine::Id { name } => p.name = name,
                UciLine::UciOk => break,
                _ => {}
            }
        }
        p.send(&format!("setoption name Threads value {}", opts.threads.max(1))).await?;
        p.send(&format!("setoption name Hash value {}", opts.hash_mb.max(16))).await?;
        p.send("setoption name UCI_ShowWDL value true").await?;
        p.send("setoption name MultiPV value 1").await?;
        p.is_ready().await?;
        if p.name.is_empty() {
            p.name = "unknown engine".into();
        }
        Ok(p)
    }

    async fn send(&mut self, cmd: &str) -> Result<(), EngineError> {
        tracing::trace!(target: "uci", "> {cmd}");
        self.stdin
            .write_all(format!("{cmd}\n").as_bytes())
            .await
            .map_err(|_| EngineError::Died)?;
        self.stdin.flush().await.map_err(|_| EngineError::Died)
    }

    async fn recv_until(&mut self, deadline: Instant) -> Result<String, EngineError> {
        let wait = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(wait, self.lines.recv()).await {
            Ok(Some(l)) => {
                tracing::trace!(target: "uci", "< {l}");
                Ok(l)
            }
            Ok(None) => Err(EngineError::Died),
            Err(_) => Err(EngineError::Protocol("timed out waiting for engine".into())),
        }
    }

    pub async fn is_ready(&mut self) -> Result<(), EngineError> {
        self.send("isready").await?;
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            if let UciLine::ReadyOk = parse_line(&self.recv_until(deadline).await?) {
                return Ok(());
            }
        }
    }

    /// Clear hash between unrelated games.
    pub async fn new_game(&mut self) -> Result<(), EngineError> {
        self.send("ucinewgame").await?;
        self.is_ready().await
    }

    /// Run one search. `on_update` receives throttled partial snapshots.
    /// On cancellation the search is stopped cleanly and `Cancelled` returned;
    /// if the engine does not answer `stop` in time the caller must drop us.
    pub async fn analyse(
        &mut self,
        req: &AnalysisRequest,
        turn: Color,
        cancel: &CancellationToken,
        mut on_update: impl FnMut(&Analysis),
    ) -> Result<Analysis, EngineError> {
        let multipv = req.multipv.clamp(1, 10);
        if multipv != self.multipv {
            self.send(&format!("setoption name MultiPV value {multipv}")).await?;
            self.multipv = multipv;
        }
        self.send(&format!("position fen {}", req.fen)).await?;
        self.send(&req.limit.go_command()).await?;

        let mut lines: BTreeMap<u32, PvLine> = BTreeMap::new();
        let mut snap = Analysis {
            fen: req.fen.clone(),
            depth: 0,
            multipv,
            lines: vec![],
            nodes: 0,
            nps: 0,
            time_ms: 0,
            best_move: None,
            terminal: None,
            done: false,
            engine: self.name.clone(),
        };
        let mut last_emit = Instant::now() - EMIT_INTERVAL;
        let mut last_depth = 0;
        let mut stopping: Option<Instant> = None;

        loop {
            let line = if let Some(deadline) = stopping {
                self.recv_until(deadline).await?
            } else {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        self.send("stop").await?;
                        stopping = Some(Instant::now() + STOP_TIMEOUT);
                        continue;
                    }
                    l = self.lines.recv() => l.ok_or(EngineError::Died)?,
                }
            };
            match parse_line(&line) {
                UciLine::Info(info) => {
                    let (Some(score), Some(depth)) = (info.score, info.depth) else { continue };
                    if info.pv.is_empty() {
                        continue;
                    }
                    let rank = info.multipv.unwrap_or(1);
                    if rank > multipv {
                        continue;
                    }
                    if info.bound != Some(Bound::Exact)
                        && lines.get(&rank).is_some_and(|l| l.depth >= depth)
                    {
                        continue;
                    }
                    let wdl = info.wdl.map(|[w, d, l]| match turn {
                        Color::White => [w, d, l],
                        Color::Black => [l, d, w],
                    });
                    lines.insert(
                        rank,
                        PvLine { rank, depth, score: score.from_pov(turn), wdl, pv: info.pv },
                    );
                    snap.nodes = info.nodes.unwrap_or(snap.nodes);
                    snap.nps = info.nps.unwrap_or(snap.nps);
                    snap.time_ms = info.time_ms.unwrap_or(snap.time_ms);
                    if rank == 1 {
                        snap.depth = depth;
                    }
                    let new_depth = snap.depth > last_depth;
                    if stopping.is_none() && (new_depth || last_emit.elapsed() >= EMIT_INTERVAL) {
                        snap.lines = lines.values().cloned().collect();
                        on_update(&snap);
                        last_emit = Instant::now();
                        last_depth = snap.depth;
                    }
                }
                UciLine::BestMove { best, .. } => {
                    if stopping.is_some() {
                        return Err(EngineError::Cancelled);
                    }
                    snap.lines = lines.into_values().collect();
                    snap.best_move = best.or_else(|| snap.lines.first().and_then(|l| l.pv.first().cloned()));
                    snap.done = true;
                    return Ok(snap);
                }
                _ => {}
            }
        }
    }

    pub async fn quit(mut self) {
        let _ = self.send("quit").await;
        let _ = tokio::time::timeout(Duration::from_secs(1), self.child.wait()).await;
    }
}

/// Mate-aware sort key: larger is better for White.
pub fn score_order(s: Score) -> i64 {
    match s {
        Score::Cp(c) => c as i64,
        Score::Mate(m) if m > 0 => 1_000_000 - m as i64,
        Score::Mate(m) => -1_000_000 - m as i64,
    }
}
