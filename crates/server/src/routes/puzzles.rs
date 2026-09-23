//! Training: the user's puzzles due for review.

use api_types::{Puzzle, PuzzleAttempt, PuzzleQueue};
use axum::Json;
use axum::extract::Path;

use crate::auth::UserState;
use crate::error::ApiResult;

pub async fn queue(UserState(state, _): UserState) -> ApiResult<Json<PuzzleQueue>> {
    let (total, due_count, learned) = state.db.puzzle_counts().await?;
    Ok(Json(PuzzleQueue { due: state.db.due_puzzles(20).await?, total, due_count, learned }))
}

pub async fn attempt(
    UserState(state, _): UserState,
    Path(id): Path<String>,
    Json(a): Json<PuzzleAttempt>,
) -> ApiResult<Json<Puzzle>> {
    Ok(Json(state.db.record_attempt(&id, a.solved).await?))
}
