use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
};
use serde::{Deserialize, Serialize};

use super::{AppState, error::ApiError};
use crate::{
    notify::{Notification, SendReport},
    scheduler::store::NotificationRecord,
};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

#[derive(Debug, Serialize)]
pub struct TestResponse {
    channel: &'static str,
    #[serde(flatten)]
    report: SendReport,
}

/// Sends a test notification to every device, so the user can check that they arrive.
pub async fn test(State(state): State<AppState>) -> Result<Json<TestResponse>, ApiError> {
    let notification = Notification {
        title: "🔔 Notifica di prova".to_owned(),
        body: "Le notifiche di InTime arrivano su questo dispositivo.".to_owned(),
        url: "/notifiche".to_owned(),
        tag: Some("test".to_owned()),
    };
    let report = state.notifier.send(&notification).await?;
    Ok(Json(TestResponse {
        channel: state.notifier.channel(),
        report,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    notifications: Vec<NotificationRecord>,
}

/// The log of sent notifications, newest first, with the name of the rule when it still exists.
pub async fn list(
    State(state): State<AppState>,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<ListResponse>, ApiError> {
    let Query(query) = query.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::InvalidRequest(format!(
            "limit must be between 1 and {MAX_LIMIT}"
        )));
    }
    Ok(Json(ListResponse {
        notifications: state.scheduler.list_notifications(limit).await?,
    }))
}
