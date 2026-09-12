//! wellbe.social — the landing page and waiting list.
//!
//! Split into a library plus a thin binary so the whole thing, database and
//! all, can be driven from an integration test rather than only from a browser.

pub mod config;
pub mod content;
pub mod error;
pub mod invite;
pub mod outbox;
pub mod signup;
pub mod templates;
pub mod web;
