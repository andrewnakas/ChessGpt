//! Background game-analysis jobs: engine pass, key moments, deep pass,
//! explanations and review, persisted as they go and broadcast to listeners.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api_types::{EloTier, Explanation, GameAnalysis, JobEvent, JobStage, JobStatus, MoveEval};
use coach::analysis::{deep_pass, engine_pass};
use coach::explain::{explain_moment, review_game};
use coach::key_moments;
use coach::prompts::{EXPLAIN_VERSION, GameContext};
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

pub struct JobHandle {
    pub tx: broadcast::Sender<JobEvent>,
    pub cancel: CancellationToken,
}

#[derive(Default)]
pub struct Jobs {
    running: Mutex<HashMap<String, Arc<JobHandle>>>,
}

impl Jobs {
    pub fn get(&self, id: &str) -> Option<Arc<JobHandle>> {
        self.running.lock().unwrap().get(id).cloned()
    }

    fn insert(&self, id: &str) -> Arc<JobHandle> {
        let (tx, _) = broadcast::channel(2048);
        let h = Arc::new(JobHandle { tx, cancel: CancellationToken::new() });
        self.running.lock().unwrap().insert(id.to_string(), h.clone());
        h
    }

    fn remove(&self, id: &str) {
        self.running.lock().unwrap().remove(id);
    }

    pub fn cancel_all(&self) {
        for h in self.running.lock().unwrap().values() {
            h.cancel.cancel();
        }
    }
}

#[derive(Clone)]
pub struct JobParams {
    pub analysis_id: String,
    pub explain: bool,
}

/// Start (or restart) an analysis job in the background.
pub fn spawn(state: AppState, p: JobParams) {
    if state.jobs.get(&p.analysis_id).is_some() {
        return;
    }
    let handle = state.jobs.insert(&p.analysis_id);
    tokio::spawn(async move {
        let id = p.analysis_id.clone();
        let res = run(&state, &p, &handle).await;
        let status = match &res {
            Ok(()) => JobStatus::Done,
            Err(_) if handle.cancel.is_cancelled() => JobStatus::Cancelled,
            Err(_) => JobStatus::Failed,
        };
        let err = res.err();
        if let Err(e) = state.db.set_analysis_status(&id, status, err.as_deref()).await {
            tracing::error!("could not record job status: {e}");
        }
        match state.db.analysis(&id).await {
            Ok(a) => {
                if let Some(e) = &err
                    && status == JobStatus::Failed
                {
                    let _ = handle.tx.send(JobEvent::Error { message: e.clone() });
                }
                let _ = handle.tx.send(JobEvent::Done { analysis: a });
            }
            Err(e) => {
                let _ = handle.tx.send(JobEvent::Error { message: e.to_string() });
            }
        }
        state.jobs.remove(&id);
    });
}

fn emit(h: &JobHandle, e: JobEvent) {
    let _ = h.tx.send(e);
}

async fn run(state: &AppState, p: &JobParams, h: &JobHandle) -> Result<(), String> {
    let db = &state.db;
    let s = |e: db::DbError| e.to_string();
    db.set_analysis_status(&p.analysis_id, JobStatus::Running, None).await.map_err(s)?;
    let analysis = db.analysis(&p.analysis_id).await.map_err(s)?;
    let game = db.game(&analysis.game_id).await.map_err(s)?;
    let tier = EloTier::from_elo(analysis.elo);
    let parsed = &game.parsed;
    let total = parsed.moves.len() as u32;

    // 1. Engine pass, persisted move by move through a writer task.
    let (mtx, mut mrx) = mpsc::unbounded_channel::<MoveEval>();
    let writer_db = db.clone();
    let writer_id = p.analysis_id.clone();
    let writer = tokio::spawn(async move {
        while let Some(m) = mrx.recv().await {
            if let Err(e) = writer_db.put_move_eval(&writer_id, &m).await {
                tracing::error!("move eval write failed: {e}");
            }
        }
    });
    let mut done = 0u32;
    let pass = engine_pass(&state.pool, parsed, tier.batch(), &h.cancel, |m| {
        done += 1;
        let _ = mtx.send(m.clone());
        emit(h, JobEvent::Move { eval: m.clone() });
        emit(h, JobEvent::Progress { stage: JobStage::Engine, done, total });
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(mtx);
    let _ = writer.await;

    db.set_start_score(&p.analysis_id, Some(pass.start_score)).await.map_err(s)?;
    db.set_accuracy(&p.analysis_id, pass.white_accuracy, pass.black_accuracy).await.map_err(s)?;
    emit(h, JobEvent::Accuracy { white: pass.white_accuracy, black: pass.black_accuracy });

    // 2. Key moments and the mistake index.
    let result = parsed.tag("Result").unwrap_or("*");
    let keys = key_moments::select(&pass.moves, analysis.user_side, result, key_moments::DEFAULT_MAX);
    db.set_key_moments(&p.analysis_id, &keys).await.map_err(s)?;
    emit(h, JobEvent::KeyMoments { plies: keys.clone() });
    db.index_mistakes(&p.analysis_id).await.map_err(s)?;

    if !p.explain || keys.is_empty() {
        return Ok(());
    }
    let provider = match state.provider().await {
        Ok(pr) => pr,
        Err(e) => {
            // Engine analysis is complete; explanations need a provider.
            emit(h, JobEvent::Error { message: e.1 });
            return Ok(());
        }
    };

    // 3. Deep pass on the key moments.
    emit(h, JobEvent::Progress { stage: JobStage::DeepPass, done: 0, total: keys.len() as u32 });
    let already: Vec<u32> = db.analysis(&p.analysis_id).await.map_err(s)?.explanations.iter().map(|e| e.ply).collect();
    let todo: Vec<u32> = keys.iter().copied().filter(|k| !already.contains(k)).collect();
    let contexts = deep_pass(&state.pool, parsed, &todo, tier.multipv(), tier.interactive(), &h.cancel)
        .await
        .map_err(|e| e.to_string())?;

    // 4. Explanations, three at a time.
    let ctx = GameContext::new(parsed, analysis.user_side, analysis.elo, game.summary.opening.clone());
    let total_expl = contexts.len() as u32;
    let mut finished = 0u32;
    let jobs = futures::stream::iter(contexts.into_iter().map(|mc| {
        let provider = provider.clone();
        let ctx = ctx.clone();
        let m = pass.moves[mc.ply as usize - 1].clone();
        async move {
            emit(h, JobEvent::ExplanationStarted { ply: mc.ply });
            let r = explain_moment(provider.as_ref(), &ctx, &m, &mc).await;
            (mc.ply, r)
        }
    }))
    .buffer_unordered(3);
    tokio::pin!(jobs);
    loop {
        let next = tokio::select! {
            _ = h.cancel.cancelled() => return Err("cancelled".into()),
            n = jobs.next() => n,
        };
        let Some((ply, r)) = next else { break };
        finished += 1;
        emit(h, JobEvent::Progress { stage: JobStage::Explaining, done: finished, total: total_expl });
        match r {
            Ok(x) => {
                let (ti, to) = (Some(x.usage.input_tokens), Some(x.usage.output_tokens));
                db.put_explanation(&p.analysis_id, &x.explanation, EXPLAIN_VERSION, &x.request, &x.response, ti, to)
                    .await
                    .map_err(s)?;
                emit(h, JobEvent::Explanation { explanation: x.explanation });
            }
            Err(e) => {
                tracing::warn!("explanation for ply {ply} failed: {e}");
                emit(h, JobEvent::ExplanationFailed { ply, message: e.to_string() });
            }
        }
    }

    // 5. Game review.
    emit(h, JobEvent::Progress { stage: JobStage::Review, done: 0, total: 1 });
    let explanations: Vec<Explanation> = db.analysis(&p.analysis_id).await.map_err(s)?.explanations;
    let moves_with_keys: Vec<MoveEval> = pass
        .moves
        .iter()
        .map(|m| MoveEval { is_key_moment: keys.contains(&m.ply), ..m.clone() })
        .collect();
    match review_game(provider.as_ref(), &ctx, &moves_with_keys, pass.white_accuracy, pass.black_accuracy, &explanations)
        .await
    {
        Ok((review, _)) => {
            db.set_review(&p.analysis_id, &review).await.map_err(s)?;
            emit(h, JobEvent::Review { review });
        }
        Err(e) => {
            tracing::warn!("review failed: {e}");
            emit(h, JobEvent::Error { message: format!("game review failed: {e}") });
        }
    }
    Ok(())
}

/// Restart analyses interrupted by a previous shutdown.
pub async fn resume_unfinished(state: &AppState) {
    match state.db.unfinished_analyses().await {
        Ok(ids) => {
            for id in ids {
                tracing::info!("resuming analysis {id}");
                spawn(state.clone(), JobParams { analysis_id: id, explain: true });
            }
        }
        Err(e) => tracing::error!("could not list unfinished analyses: {e}"),
    }
}

/// Snapshot for a newly connected listener.
pub async fn snapshot(state: &AppState, id: &str) -> Result<GameAnalysis, db::DbError> {
    state.db.analysis(id).await
}
