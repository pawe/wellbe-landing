//! "We inform you when we are ready for you."
//!
//! The promise on the front page needs something that can actually keep it.
//! This is that: a command that takes people off the waiting list, oldest
//! first, and queues the message telling them so.
//!
//!   wellbe-landing invite --count 50      # the next fifty in line
//!   wellbe-landing invite paul@example.com
//!   wellbe-landing invite --all
//!   wellbe-landing invite --count 50 --dry-run

use sqlx::PgPool;
use uuid::Uuid;

use crate::outbox;

pub enum Who {
    Next(i64),
    One(String),
    Everyone,
}

pub struct Plan {
    pub who: Who,
    pub dry_run: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum InviteError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("usage: wellbe-landing invite [--all | --count N | <email>] [--dry-run]")]
    Usage,
}

impl Plan {
    pub fn from_args(args: &[String]) -> Result<Self, InviteError> {
        let dry_run = args.iter().any(|arg| arg == "--dry-run");
        let rest: Vec<&String> = args.iter().filter(|arg| *arg != "--dry-run").collect();

        let who = match rest.split_first() {
            Some((first, tail)) if *first == "--all" && tail.is_empty() => Who::Everyone,
            Some((first, tail)) if *first == "--count" => {
                let raw = tail.first().ok_or(InviteError::Usage)?;
                Who::Next(raw.parse().map_err(|_| InviteError::Usage)?)
            }
            Some((first, tail)) if tail.is_empty() && !first.starts_with("--") => {
                let email = crate::signup::normalise_email(first).ok_or(InviteError::Usage)?;
                Who::One(email)
            }
            _ => return Err(InviteError::Usage),
        };

        Ok(Self { who, dry_run })
    }
}

/// Returns the people who were invited.
pub async fn run(pool: &PgPool, plan: &Plan, base_url: &str) -> Result<Vec<String>, InviteError> {
    let mut tx = pool.begin().await?;

    // Oldest confirmed first, and never anybody twice. `for update skip locked`
    // so that two people running this at once cannot double-invite.
    let selected: Vec<(Uuid, String, String)> = match &plan.who {
        Who::One(email) => {
            sqlx::query_as(
                "select id, name, email from signup
                  where email = $1 and confirmed_at is not null and invited_at is null
                  for update skip locked",
            )
            .bind(email)
            .fetch_all(&mut *tx)
            .await?
        }
        Who::Next(count) => {
            sqlx::query_as(
                "select id, name, email from signup
                  where confirmed_at is not null and invited_at is null
                  order by confirmed_at
                  limit $1
                  for update skip locked",
            )
            .bind(count)
            .fetch_all(&mut *tx)
            .await?
        }
        Who::Everyone => {
            sqlx::query_as(
                "select id, name, email from signup
                  where confirmed_at is not null and invited_at is null
                  order by confirmed_at
                  for update skip locked",
            )
            .fetch_all(&mut *tx)
            .await?
        }
    };

    if plan.dry_run {
        // Roll back rather than commit: nothing was written, nothing is queued.
        tx.rollback().await?;
        return Ok(selected.into_iter().map(|(_, _, email)| email).collect());
    }

    let mut invited = Vec::with_capacity(selected.len());

    for (id, name, email) in selected {
        sqlx::query("update signup set invited_at = now() where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        outbox::enqueue(&mut tx, outbox::Message::ready(id, &email, &name, base_url)).await?;
        invited.push(email);
    }

    tx.commit().await?;
    Ok(invited)
}
