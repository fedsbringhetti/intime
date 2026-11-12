use std::sync::Arc;

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::json;

use crate::{
    departures::DeparturesError, favorites::FavoritesError, notify::NotifyError, push::PushError,
    rules::RulesError, scheduler::store::SchedulerStoreError, stops::IndexError,
    upstream::UpstreamError,
};

/// Errors returned to API clients as `{"error": {"code": ..., "message": ...}}`.
#[derive(Debug)]
pub enum ApiError {
    InvalidRequest(String),
    /// Missing or wrong bearer token.
    Unauthorized,
    StopNotFound(String),
    FavoriteNotFound(i64),
    RuleNotFound(i64),
    /// The action exists only for summary rules.
    NotSummary(i64),
    UpstreamTimeout,
    Upstream(String),
    /// The stop index has not been synced yet.
    IndexNotReady,
    /// No VAPID key is configured, so there is no Web Push.
    PushDisabled,
    Internal,
}

/// The `{"code", "message"}` part of an error, also used inside larger responses (one
/// favorite whose departures failed).
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn into_parts(self) -> (StatusCode, ErrorBody) {
        let (status, code, message) = match self {
            Self::InvalidRequest(message) => (StatusCode::BAD_REQUEST, "invalid_request", message),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or invalid bearer token".to_owned(),
            ),
            Self::StopNotFound(stop) => (
                StatusCode::NOT_FOUND,
                "stop_not_found",
                format!("Stop {stop} not found"),
            ),
            Self::FavoriteNotFound(id) => (
                StatusCode::NOT_FOUND,
                "favorite_not_found",
                format!("Favorite {id} not found"),
            ),
            Self::RuleNotFound(id) => (
                StatusCode::NOT_FOUND,
                "rule_not_found",
                format!("Rule {id} not found"),
            ),
            Self::NotSummary(id) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "not_a_summary",
                format!("Rule {id} is not a summary rule"),
            ),
            Self::UpstreamTimeout => (
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
                "The upstream service did not answer in time".to_owned(),
            ),
            Self::Upstream(message) => (StatusCode::BAD_GATEWAY, "upstream_error", message),
            Self::IndexNotReady => (
                StatusCode::SERVICE_UNAVAILABLE,
                "index_not_ready",
                "The stop index is still being downloaded, try again shortly".to_owned(),
            ),
            Self::PushDisabled => (
                StatusCode::SERVICE_UNAVAILABLE,
                "push_disabled",
                "Web Push is not configured on the server (no VAPID key)".to_owned(),
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal error".to_owned(),
            ),
        };
        (status, ErrorBody { code, message })
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = self.into_parts();
        let body = json!({ "error": error });
        let mut response = (status, Json(body)).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

impl From<Arc<DeparturesError>> for ApiError {
    fn from(err: Arc<DeparturesError>) -> Self {
        match &*err {
            DeparturesError::StopNotFound(stop) => Self::StopNotFound(stop.clone()),
            DeparturesError::Upstream(err) => {
                tracing::warn!("{err}");
                match err {
                    UpstreamError::Timeout => Self::UpstreamTimeout,
                    err => Self::Upstream(err.to_string()),
                }
            }
        }
    }
}

impl From<IndexError> for ApiError {
    fn from(err: IndexError) -> Self {
        tracing::error!("stop index: {err}");
        Self::Internal
    }
}

impl From<FavoritesError> for ApiError {
    fn from(err: FavoritesError) -> Self {
        tracing::error!("favorites: {err}");
        Self::Internal
    }
}

impl From<RulesError> for ApiError {
    fn from(err: RulesError) -> Self {
        tracing::error!("rules: {err}");
        Self::Internal
    }
}

impl From<PushError> for ApiError {
    fn from(err: PushError) -> Self {
        tracing::error!("push subscriptions: {err}");
        Self::Internal
    }
}

impl From<NotifyError> for ApiError {
    fn from(err: NotifyError) -> Self {
        tracing::error!("notifications: {err}");
        Self::Internal
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!("database: {err}");
        Self::Internal
    }
}

impl From<SchedulerStoreError> for ApiError {
    fn from(err: SchedulerStoreError) -> Self {
        tracing::error!("scheduler: {err}");
        Self::Internal
    }
}
