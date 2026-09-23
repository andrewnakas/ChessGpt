use std::time::Instant;

use api_types::{Meta, Provider, ProviderInput, ProviderTestResult, Settings, SettingsInput};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::error::{ApiResult, AppError};
use crate::state::AppState;

pub async fn get(State(state): State<AppState>) -> ApiResult<Json<Settings>> {
    Ok(Json(state.db.settings().await?))
}

pub async fn put(State(state): State<AppState>, Json(s): Json<SettingsInput>) -> ApiResult<Json<Settings>> {
    Ok(Json(state.db.put_settings(&s).await?))
}

pub async fn providers(State(state): State<AppState>) -> ApiResult<Json<Vec<Provider>>> {
    Ok(Json(state.db.list_providers().await?))
}

fn check(input: &ProviderInput, has_stored_key: bool) -> ApiResult<()> {
    let key_given = input.api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
    if input.kind.needs_key() && !key_given && !has_stored_key {
        return Err(AppError::bad_request(format!("{} needs an API key", input.kind.as_str())));
    }
    if let Some(u) = input.base_url.as_deref().filter(|u| !u.trim().is_empty())
        && !(u.starts_with("http://") || u.starts_with("https://"))
    {
        return Err(AppError::bad_request("base URL must start with http:// or https://"));
    }
    Ok(())
}

pub async fn create_provider(State(state): State<AppState>, Json(i): Json<ProviderInput>) -> ApiResult<Json<Provider>> {
    check(&i, false)?;
    Ok(Json(state.db.create_provider(&i).await?))
}

pub async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(i): Json<ProviderInput>,
) -> ApiResult<Json<Provider>> {
    let existing = state.db.provider(&id).await?;
    check(&i, existing.api_key.is_some() && i.api_key.as_deref() != Some(""))?;
    Ok(Json(state.db.update_provider(&id, &i).await?))
}

pub async fn delete_provider(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    state.db.delete_provider(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn test_provider(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<ProviderTestResult>> {
    let rec = state.db.provider(&id).await?;
    let started = Instant::now();
    let p = llm::build(llm::ProviderConfig {
        kind: rec.provider.kind,
        base_url: rec.provider.base_url,
        model: rec.provider.model,
        api_key: rec.api_key,
    });
    let res = match p {
        Ok(p) => llm::ping(p.as_ref()).await,
        Err(e) => Err(e),
    };
    let latency_ms = started.elapsed().as_millis() as u64;
    Ok(Json(match res {
        Ok(msg) => ProviderTestResult { ok: true, message: msg, latency_ms },
        Err(e) => ProviderTestResult { ok: false, message: e.to_string(), latency_ms },
    }))
}

pub async fn meta(State(state): State<AppState>) -> ApiResult<Json<Meta>> {
    let cfg = state.pool.config();
    Ok(Json(Meta {
        version: env!("CARGO_PKG_VERSION").into(),
        engine: state.pool.engine_name().into(),
        engine_threads: cfg.threads,
        engine_workers: cfg.workers as u32,
        mode: state.config.mode.clone(),
        has_provider: state.db.default_provider().await?.is_some(),
    }))
}
