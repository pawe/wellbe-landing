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
    /// The token from the confirmation mail, which is also how the form on this
    /// page posts back and how somebody returns to it later.
    pub token: String,
    /// True only on the click that did the confirming, so a return visit does
    /// not greet somebody as though they had just arrived.
    pub just_confirmed: bool,
    pub relationships: Vec<Relationship>,
    pub saved_contacts: Vec<crate::signup::SavedContact>,
    pub saved_watches: Vec<crate::signup::SavedWatch>,
    /// Blank rows, or what was typed if a submission bounced.
    pub form: RawForm,
    pub error: Option<String>,
    /// Set straight after a successful add, to say what happened.
    pub added: Option<crate::signup::Added>,
}

impl ConfirmedPage {
    pub fn new(
        person: crate::signup::Person,
        token: String,
        just_confirmed: bool,
        relationships: Vec<Relationship>,
        form: RawForm,
    ) -> Self {
        Self {
            name: person.name,
            token,
            just_confirmed,
            relationships,
            saved_contacts: person.contacts,
            saved_watches: person.watches,
            form: form.padded(),
            error: None,
            added: None,
        }
    }

    pub fn with_error(mut self, message: String) -> Self {
        self.error = Some(message);
        self
    }

    pub fn with_added(mut self, added: crate::signup::Added) -> Self {
        self.added = Some(added);
        self
    }

    /// Whether there is anything to show back to them yet.
    pub fn has_saved(&self) -> bool {
        !self.saved_contacts.is_empty() || !self.saved_watches.is_empty()
    }
}

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorPage {
    pub status: u16,
    pub message: String,
}
