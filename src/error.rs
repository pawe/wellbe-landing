use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error")]
    Database(#[from] sqlx::Error),

    #[error("template error")]
    Render(#[from] askama::Error),

    #[error("not found")]
    NotFound,

    /// Something the visitor can fix, phrased for the visitor.
    #[error("{0}")]
    Invalid(String),
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Database(_) | Self::Render(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
        }
    }

    /// What we are willing to say out loud. Internal failures get a generic
    /// line; the visitor did nothing wrong and the details belong in the log.
    pub fn public_message(&self) -> String {
        match self {
            Self::Database(_) | Self::Render(_) => {
                "Something broke on our side. That is our fault, not yours — please try again in a \
                 moment."
                    .to_owned()
            }
            Self::NotFound => "We could not find that page.".to_owned(),
            Self::Invalid(message) => message.clone(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();

        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        } else {
            tracing::debug!(error = %self, "request rejected");
        }

        let page = crate::templates::ErrorPage {
            status: status.as_u16(),
            message: self.public_message(),
        };

        match askama::Template::render(&page) {
            Ok(html) => (status, axum::response::Html(html)).into_response(),
            Err(_) => (status, self.public_message()).into_response(),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
