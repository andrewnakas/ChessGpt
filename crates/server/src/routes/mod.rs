pub mod chat;
pub mod engine;
pub mod games;
pub mod settings;

use axum::Router;
use axum::routing::{get, post, put};

use crate::state::AppState;
use crate::{assets, auth, mcp, oauth};

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/session", get(auth::session))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/meta", get(settings::meta))
        .route("/engine/analyse", get(engine::analyse))
        .route("/games", get(games::list))
        .route("/games/import", post(games::import))
        .route("/games/{id}", get(games::detail).delete(games::delete))
        .route("/games/{id}/side", put(games::set_side))
        .route("/games/{id}/analyse", post(games::analyse))
        .route("/games/{id}/share", post(games::share))
        .route("/share/{token}", get(games::shared))
        .route("/share/{token}/claim", post(games::claim))
        .route("/analyses/{id}", get(games::get_analysis))
        .route("/analyses/{id}/events", get(games::events))
        .route("/analyses/{id}/cancel", post(games::cancel))
        .route("/analyses/{id}/explain/{ply}", post(games::explain_ply))
        .route("/settings", get(settings::get).put(settings::put))
        .route("/providers", get(settings::providers).post(settings::create_provider))
        .route("/providers/{id}", put(settings::update_provider).delete(settings::delete_provider))
        .route("/providers/{id}/test", post(settings::test_provider))
        .route("/connections", get(settings::connections))
        .route("/connections/{client_id}", axum::routing::delete(settings::disconnect))
        .route("/chat/threads", get(chat::list).post(chat::create))
        .route("/chat/threads/{id}", get(chat::detail).delete(chat::delete))
        .route("/chat/threads/{id}/messages", post(chat::send));
    Router::new()
        .nest("/api", api)
        .route("/.well-known/oauth-protected-resource", get(oauth::protected_resource))
        .route("/.well-known/oauth-protected-resource/mcp", get(oauth::protected_resource))
        .route("/.well-known/oauth-authorization-server", get(oauth::authorization_server))
        .route("/.well-known/openai-apps-challenge", get(oauth::openai_challenge))
        .route("/oauth/register", post(oauth::register))
        .route("/oauth/authorize", get(oauth::authorize).post(oauth::decide))
        .route("/oauth/token", post(oauth::token))
        .route("/oauth/revoke", post(oauth::revoke))
        .nest_service(
            "/mcp",
            tower::ServiceBuilder::new()
                .layer(axum::middleware::from_fn_with_state(state.clone(), mcp::require_token))
                .service(mcp::service(&state)),
        )
        .route("/auth/lichess", get(auth::lichess_start))
        .route("/auth/lichess/callback", get(auth::lichess_callback))
        .fallback(assets::serve)
        .layer(tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
            .allow_headers(tower_http::cors::Any))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}
