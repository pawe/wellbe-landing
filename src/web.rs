use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_http::{
    compression::CompressionLayer, limit::RequestBodyLimitLayer, services::ServeDir,
    set_header::SetResponseHeaderLayer, trace::TraceLayer,
};

use crate::{
    config::Config,
    error::{AppError, AppResult},
    signup::{self, RawForm},
    templates::{ConfirmedPage, IndexPage, Page, Relationship, ThanksPage},
};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
}

pub fn router(state: AppState) -> Router {
    // No third-party anything. The page loads its own stylesheet, its own
    // script and nothing else — which is the only version of "we don't track
    // you" that a visitor can check for themselves with view-source.
    let csp = "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; \
               form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

    Router::new()
        .route("/", get(index).post(create_signup))
        .route("/confirm/{token}", get(confirm))
        .route("/health", get(health))
        .nest_service("/static", ServeDir::new("static").precompressed_gzip())
        .fallback(not_found)
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(csp),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(CompressionLayer::new())
        // A signup with fifty contacts is a few kilobytes. Anything past this
        // is not somebody filling in a form.
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Debug, Deserialize, Default)]
pub struct IndexQuery {
    /// `?via=a-friend` — so we can say thank you to whoever sent you.
    pub via: Option<String>,
}

async fn relationships(pool: &PgPool) -> AppResult<Vec<Relationship>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("select key, label from relationship_kind order by sort_order")
            .fetch_all(pool)
            .await?;

    Ok(rows
        .into_iter()
        .map(|(key, label)| Relationship { key, label })
        .collect())
}

/// How many people are waiting — but only once that number says something
/// other than "you would be early". Below the threshold we show nothing.
async fn waiting_count(state: &AppState) -> Option<i64> {
    match signup::confirmed_count(&state.pool).await {
        Ok(count) if count >= state.config.counter_threshold => Some(count),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(?error, "could not count signups");
            None
        }
    }
}

async fn index(
    State(state): State<AppState>,
    Query(query): Query<IndexQuery>,
) -> AppResult<Response> {
    let relationships = relationships(&state.pool).await?;
    let waiting = waiting_count(&state).await;

    Ok(Page(IndexPage::new(
        relationships,
        waiting,
        RawForm::blank(query.via),
    ))
    .into_response())
}

async fn create_signup(State(state): State<AppState>, body: String) -> AppResult<Response> {
    let form = RawForm::from_body(&body);

    let submission = match form.validate() {
        Ok(submission) => submission,
        Err(error @ AppError::Invalid(_)) => {
            // Hand the page back with the message *and* everything they typed.
            let relationships = relationships(&state.pool).await?;
            let waiting = waiting_count(&state).await;
            let page =
                IndexPage::new(relationships, waiting, form).with_error(error.public_message());
            return Ok((StatusCode::UNPROCESSABLE_ENTITY, Page(page)).into_response());
        }
        Err(other) => return Err(other),
    };

    signup::record(&state.pool, &submission, &state.config.base_url).await?;

    // The same page whatever the outcome: whether an address is already on the
    // list is between us and whoever reads that mailbox.
    Ok(Page(ThanksPage {
        email: submission.email,
    })
    .into_response())
}

async fn confirm(State(state): State<AppState>, Path(token): Path<String>) -> AppResult<Response> {
    match signup::confirm(&state.pool, &token).await? {
        Some(name) => Ok(Page(ConfirmedPage { name }).into_response()),
        None => Err(AppError::NotFound),
    }
}

async fn health(State(state): State<AppState>) -> Response {
    match sqlx::query("select 1").execute(&state.pool).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(error) => {
            tracing::error!(?error, "health check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "database unreachable").into_response()
        }
    }
}

async fn not_found() -> AppError {
    AppError::NotFound
}
