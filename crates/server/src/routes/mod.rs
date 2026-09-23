pub mod chat;
pub mod engine;
pub mod games;
pub mod settings;

use axum::Router;
use axum::routing::{get, post, put};

use crate::state::AppState;
use crate::{assets, auth};

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/session", get(auth::session))
        .route("/login", post(auth::login))
        .route("/meta", get(settings::meta))
        .route("/engine/analyse", get(engine::analyse))
        .route("/games", get(games::list))
        .route("/games/import", post(games::import))
        .route("/games/{id}", get(games::detail).delete(games::delete))
        .route("/games/{id}/side", put(games::set_side))
        .route("/games/{id}/analyse", post(games::analyse))
        .route("/analyses/{id}", get(games::get_analysis))
        .route("/analyses/{id}/events", get(games::events))
        .route("/analyses/{id}/cancel", post(games::cancel))
        .route("/analyses/{id}/explain/{ply}", post(games::explain_ply))
        .route("/settings", get(settings::get).put(settings::put))
        .route("/providers", get(settings::providers).post(settings::create_provider))
        .route("/providers/{id}", put(settings::update_provider).delete(settings::delete_provider))
        .route("/providers/{id}/test", post(settings::test_provider))
        .route("/chat/threads", get(chat::list).post(chat::create))
        .route("/chat/threads/{id}", get(chat::detail).delete(chat::delete))
        .route("/chat/threads/{id}/messages", post(chat::send));
    Router::new()
        .nest("/api", api)
        .fallback(assets::serve)
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth::require_session))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}
