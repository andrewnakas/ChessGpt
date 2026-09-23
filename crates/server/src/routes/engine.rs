use std::convert::Infallible;

use api_types::{EngineAnalysis, EngineEvent, LineDto};
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use chess_core::position::{parse_fen, pv_to_san};
use engine::{Analysis, AnalysisRequest, Limit, Priority};
use futures::Stream;
use serde::Deserialize;

use crate::error::ApiResult;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AnalyseQuery {
    fen: String,
    multipv: Option<u32>,
    depth: Option<u32>,
    movetime_ms: Option<u64>,
}

pub fn to_dto(a: &Analysis) -> EngineAnalysis {
    let pos = parse_fen(&a.fen).ok();
    EngineAnalysis {
        fen: a.fen.clone(),
        depth: a.depth,
        lines: a
            .lines
            .iter()
            .map(|l| LineDto {
                rank: l.rank,
                depth: l.depth,
                score: l.score,
                wdl: l.wdl,
                pv_uci: l.pv.clone(),
                pv_san: pos.as_ref().map(|p| pv_to_san(p, &l.pv)).unwrap_or_default(),
            })
            .collect(),
        nodes: a.nodes,
        nps: a.nps,
        time_ms: a.time_ms,
        terminal: a.terminal,
        done: a.done,
        engine: a.engine.clone(),
    }
}

fn event(e: &EngineEvent) -> Result<Event, Infallible> {
    Ok(Event::default().data(serde_json::to_string(e).unwrap_or_default()))
}

/// Live analysis of one position, streamed as it deepens. Closing the stream
/// stops the search.
pub async fn analyse(
    State(state): State<AppState>,
    Query(q): Query<AnalyseQuery>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let limit = Limit {
        depth: Some(q.depth.unwrap_or(24).clamp(1, 60)),
        movetime_ms: Some(q.movetime_ms.unwrap_or(15_000).clamp(100, 120_000)),
        nodes: None,
    };
    let handle = state
        .pool
        .analyse_stream(AnalysisRequest {
            fen: q.fen,
            multipv: q.multipv.unwrap_or(3).clamp(1, 5),
            limit,
            priority: Priority::Interactive,
        })
        .await?;
    let stream = async_stream(move |tx| async move {
        // Take the whole handle: closures capture disjoint fields, and leaving
        // the drop guard behind would cancel the search when this handler returns.
        let mut handle = handle;
        loop {
            tokio::select! {
                u = handle.updates.recv() => match u {
                    Some(a) => { if tx.send(EngineEvent::Update { analysis: to_dto(&a) }).await.is_err() { return; } }
                    None => break,
                },
                r = &mut handle.result => {
                    let ev = match r {
                        Ok(Ok(a)) => EngineEvent::Done { analysis: to_dto(&a) },
                        Ok(Err(e)) => EngineEvent::Error { message: e.to_string() },
                        Err(_) => EngineEvent::Error { message: "engine stopped".into() },
                    };
                    let _ = tx.send(ev).await;
                    return;
                }
            }
        }
        let ev = match (&mut handle.result).await {
            Ok(Ok(a)) => EngineEvent::Done { analysis: to_dto(&a) },
            Ok(Err(e)) => EngineEvent::Error { message: e.to_string() },
            Err(_) => EngineEvent::Error { message: "engine stopped".into() },
        };
        let _ = tx.send(ev).await;
    });
    Ok(Sse::new(futures::StreamExt::map(stream, |e| event(&e))).keep_alive(KeepAlive::default()))
}

/// Turn a producer closure into a stream via a bounded channel. The producer
/// is dropped (and with it any engine handle) when the client disconnects.
pub fn async_stream<T, F, Fut>(f: F) -> impl Stream<Item = T>
where
    T: Send + 'static,
    F: FnOnce(tokio::sync::mpsc::Sender<T>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let task = tokio::spawn(f(tx));
    let rx = tokio_stream::wrappers::ReceiverStream::new(rx);
    AbortOnDrop { inner: rx, task }
}

struct AbortOnDrop<S> {
    inner: S,
    task: tokio::task::JoinHandle<()>,
}

impl<S> Drop for AbortOnDrop<S> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl<S: Stream + Unpin> Stream for AbortOnDrop<S> {
    type Item = S::Item;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.inner).poll_next(cx)
    }
}
