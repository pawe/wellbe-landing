//! Outgoing mail, written to the database inside the transaction that caused
//! it and drained afterwards.
//!
//! Doing it this way means a message can never be sent for something that was
//! rolled back, and a committed action can never silently fail to notify
//! anyone: the row is simply still there, unsent, waiting to be retried.

use std::time::Duration;

use serde_json::json;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Message {
    pub kind: &'static str,
    pub signup_id: Option<Uuid>,
    pub to_email: String,
    pub subject: String,
    pub body: String,
}

const SIGN_OFF: &str = "\n\n— the wellbe.social crew\n\nWe are a democratically organised non-profit. \
You are on this list because you typed your address into wellbe.social. \
Reply to this mail and a person will read it.";

impl Message {
    pub fn confirm(signup_id: Uuid, to: &str, name: &str, base_url: &str, token: &str) -> Self {
        Self {
            kind: "confirm",
            signup_id: Some(signup_id),
            to_email: to.to_owned(),
            subject: "One click and you are on the wellbe.social list".to_owned(),
            body: format!(
                "Hello {name},\n\n\
                 Somebody — we hope you — put this address on the wellbe.social waiting list.\n\n\
                 Confirm it here and you are in:\n\n  {base_url}/confirm/{token}\n\n\
                 If that was not you, do nothing at all. Without that click we send you nothing \
                 else and quietly forget the whole thing.{SIGN_OFF}"
            ),
        }
    }

    pub fn already_confirmed(signup_id: Uuid, to: &str, name: &str) -> Self {
        Self {
            kind: "welcome",
            signup_id: Some(signup_id),
            to_email: to.to_owned(),
            subject: "You are already on the wellbe.social list".to_owned(),
            body: format!(
                "Hello {name},\n\n\
                 You filled the form in again — no harm done, and nothing has changed. \
                 Your place on the list is where it was, and we will write to you when we are \
                 ready for you.\n\n\
                 If you wanted to change something you told us, or you want it all deleted, \
                 just reply to this message and say so.{SIGN_OFF}"
            ),
        }
    }

    pub fn invite(signup_id: Uuid, to: &str, contact_name: &str, from_name: &str) -> Self {
        Self {
            kind: "invite",
            signup_id: Some(signup_id),
            to_email: to.to_owned(),
            subject: format!("{from_name} put their name down for wellbe.social"),
            body: format!(
                "Hello {contact_name},\n\n\
                 {from_name} joined the waiting list for wellbe.social and asked us to let you \
                 know. That is the whole message — there is nothing you have to do.\n\n\
                 wellbe.social is a non-profit building a place to keep up with the people in \
                 your life without being farmed for attention. It is not ready yet. If you would \
                 like to be told when it is: https://wellbe.social\n\n\
                 We received your address from {from_name} for this one message. We are not \
                 adding you to anything, and unless you sign up yourself you will not hear from \
                 us again.{SIGN_OFF}"
            ),
        }
    }

    pub fn watch_match(
        signup_id: Uuid,
        to: &str,
        watcher_name: &str,
        matched_name: &str,
        matched_email: &str,
    ) -> Self {
        Self {
            kind: "watch_match",
            signup_id: Some(signup_id),
            to_email: to.to_owned(),
            subject: format!("{matched_name} is on the wellbe.social list too"),
            body: format!(
                "Hello {watcher_name},\n\n\
                 You asked us to tell you if {matched_email} ever signed up. {matched_name} has, \
                 and agreed to be findable by people who already have their address — so here we \
                 are.\n\n\
                 We told them nothing about you.{SIGN_OFF}"
            ),
        }
    }

    pub fn ready(signup_id: Uuid, to: &str, name: &str, base_url: &str) -> Self {
        Self {
            kind: "ready",
            signup_id: Some(signup_id),
            to_email: to.to_owned(),
            subject: "We are ready for you".to_owned(),
            body: format!(
                "Hello {name},\n\n\
                 We said we would write when we were ready for you. We are.\n\n  {base_url}\n\
                 {SIGN_OFF}"
            ),
        }
    }
}

/// Queue a message as part of an ongoing transaction.
pub async fn enqueue(tx: &mut Transaction<'_, Postgres>, message: Message) -> sqlx::Result<()> {
    sqlx::query(
        "insert into outbox (kind, signup_id, to_email, payload)
         values ($1, $2, $3, $4)",
    )
    .bind(message.kind)
    .bind(message.signup_id)
    .bind(&message.to_email)
    .bind(json!({ "subject": message.subject, "body": message.body }))
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Somewhere for a message to go.
///
/// There is one implementation today and it writes to the log. Putting SMTP
/// behind this trait is the next commit, not a rewrite — and it means the whole
/// path from form to delivery can be exercised in a test without a mail server.
pub trait Mailer: Send + Sync + 'static {
    fn deliver(&self, to: &str, subject: &str, body: &str) -> Result<(), String>;
}

pub struct LogMailer;

impl Mailer for LogMailer {
    fn deliver(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        tracing::info!(to, subject, "outbox: would send\n{body}");
        Ok(())
    }
}

const MAX_ATTEMPTS: i32 = 8;
const BATCH: i64 = 20;

/// Drain one batch. Returns how many rows were handled.
pub async fn drain_once(pool: &PgPool, mailer: &dyn Mailer) -> sqlx::Result<usize> {
    let mut tx = pool.begin().await?;

    // `skip locked` so that several workers can drain in parallel without ever
    // sending the same message twice.
    let rows = sqlx::query(
        "select id, to_email, payload, attempts from outbox
          where sent_at is null and send_after <= now() and attempts < $1
          order by send_after
          limit $2
          for update skip locked",
    )
    .bind(MAX_ATTEMPTS)
    .bind(BATCH)
    .fetch_all(&mut *tx)
    .await?;

    let mut handled = 0;

    for row in rows {
        let id: Uuid = row.get("id");
        let to: Option<String> = row.get("to_email");
        let payload: serde_json::Value = row.get("payload");
        let attempts: i32 = row.get("attempts");

        let subject = payload
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let body = payload.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let to = to.unwrap_or_default();

        match mailer.deliver(&to, subject, body) {
            Ok(()) => {
                sqlx::query(
                    "update outbox set sent_at = now(), attempts = attempts + 1 where id = $1",
                )
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
            Err(error) => {
                // Back off: 1, 2, 4, 8 ... minutes, capped by MAX_ATTEMPTS.
                let delay_minutes = 1i64 << attempts.clamp(0, 6);
                sqlx::query(
                    "update outbox
                        set attempts = attempts + 1,
                            last_error = $2,
                            send_after = now() + make_interval(mins => $3)
                      where id = $1",
                )
                .bind(id)
                .bind(&error)
                .bind(delay_minutes as i32)
                .execute(&mut *tx)
                .await?;
                tracing::warn!(%id, error, "outbox delivery failed");
            }
        }

        handled += 1;
    }

    tx.commit().await?;
    Ok(handled)
}

/// Background loop. Polls rather than listens, because at this volume a query
/// every few seconds is cheaper than the machinery to avoid it.
pub async fn run_worker(pool: PgPool, mailer: impl Mailer) {
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        match drain_once(&pool, &mailer).await {
            Ok(0) => {}
            Ok(count) => tracing::debug!(count, "outbox drained"),
            Err(error) => tracing::error!(?error, "outbox worker failed"),
        }
    }
}
