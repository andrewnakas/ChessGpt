use api_types::ApiError;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub struct AppError(pub StatusCode, pub String);

pub type ApiResult<T> = Result<T, AppError>;

impl AppError {
    pub fn bad_request(m: impl Into<String>) -> AppError {
        AppError(StatusCode::BAD_REQUEST, m.into())
    }
    pub fn not_found(m: impl Into<String>) -> AppError {
        AppError(StatusCode::NOT_FOUND, m.into())
    }
    pub fn unavailable(m: impl Into<String>) -> AppError {
        AppError(StatusCode::SERVICE_UNAVAILABLE, m.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if self.0.is_server_error() {
            tracing::error!("{}", self.1);
        }
        (self.0, Json(ApiError { error: self.1 })).into_response()
    }
}

impl From<db::DbError> for AppError {
    fn from(e: db::DbError) -> AppError {
        match e {
            db::DbError::NotFound => AppError::not_found("not found"),
            db::DbError::Invalid(m) => AppError::bad_request(m),
            e => AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
}

impl From<engine::EngineError> for AppError {
    fn from(e: engine::EngineError) -> AppError {
        match e {
            engine::EngineError::BadFen(m) => AppError::bad_request(m),
            e => AppError::unavailable(e.to_string()),
        }
    }
}

impl From<importers::ImportError> for AppError {
    fn from(e: importers::ImportError) -> AppError {
        use importers::ImportError as E;
        let code = match &e {
            E::UserNotFound(_) | E::GameNotFound(_) => StatusCode::NOT_FOUND,
            E::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            E::BadGameId(_) | E::LichessTokenRequired | E::LichessBadToken => StatusCode::BAD_REQUEST,
            E::Http(..) => StatusCode::BAD_GATEWAY,
        };
        AppError(code, e.to_string())
    }
}

impl From<llm::LlmError> for AppError {
    fn from(e: llm::LlmError) -> AppError {
        AppError(StatusCode::BAD_GATEWAY, e.to_string())
    }
}
