use server::config::Config;
use server::{build_state, jobs, routes};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=warn")),
        )
        .init();
    let config = Config::from_env();
    let bind = config.bind;
    let dev = config.dev;
    let state = build_state(config).await?;
    tracing::info!(
        "data in {}, engine {} ({} workers x {} threads)",
        state.config.data_dir.display(),
        state.pool.engine_name(),
        state.pool.config().workers,
        state.pool.config().threads
    );
    jobs::resume_unfinished(&state).await;
    server::sync::spawn_periodic(state.clone());

    let app = routes::router(state.clone());
    let listener = tokio::net::TcpListener::bind(bind).await?;
    if dev {
        tracing::info!("API on http://{bind} (open the Vite dev server at http://localhost:5173)");
    } else {
        tracing::info!("chessgpt running at http://{bind}");
    }
    let shutdown_state = state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutting down");
            shutdown_state.jobs.cancel_all();
            shutdown_state.pool.shutdown();
        })
        .await?;
    Ok(())
}
