use askama::Template;
use axum::response::{IntoResponse, Response};

use crate::{content::Principle, signup::RawForm};

/// Wraps any template so it can be returned straight from a handler.
/// (Askama stopped shipping its own axum integration; this is all it was.)
pub struct Page<T>(pub T);

impl<T: Template> IntoResponse for Page<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => axum::response::Html(html).into_response(),
            Err(error) => crate::error::AppError::Render(error).into_response(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub key: String,
    pub label: String,
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexPage {
    pub principles: &'static [Principle],
    pub slogan_lead: &'static str,
    pub slogan_rest: &'static str,
    pub hero_intro: &'static str,
    pub footnote: &'static str,
    pub governance: &'static str,
    pub signup_promise: &'static str,
    pub relationships: Vec<Relationship>,
    /// Only shown once enough people are waiting for a number to mean anything.
    pub waiting: Option<i64>,
    /// Set when a submission bounced back; the form is re-rendered with it.
    pub error: Option<String>,
    pub form: RawForm,
}

impl IndexPage {
    pub fn new(relationships: Vec<Relationship>, waiting: Option<i64>, form: RawForm) -> Self {
        Self {
            principles: crate::content::PRINCIPLES,
            slogan_lead: crate::content::SLOGAN_LEAD,
            slogan_rest: crate::content::SLOGAN_REST,
            hero_intro: crate::content::HERO_INTRO,
            footnote: crate::content::FOOTNOTE,
            governance: crate::content::GOVERNANCE,
            signup_promise: crate::content::SIGNUP_PROMISE,
            relationships,
            waiting,
            error: None,
            form: form.padded(),
        }
    }

    pub fn with_error(mut self, message: String) -> Self {
        self.error = Some(message);
        self
    }
}

#[derive(Template)]
#[template(path = "thanks.html")]
pub struct ThanksPage {
    pub email: String,
}

#[derive(Template)]
#[template(path = "confirmed.html")]
pub struct ConfirmedPage {
    pub name: String,
}

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorPage {
    pub status: u16,
    pub message: String,
}
