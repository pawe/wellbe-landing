//! Parsing, validating and storing what somebody typed into the form.

use rand::RngExt;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    outbox,
};

/// Upper bounds. Not because we expect anyone to hit them, but because an open
/// text field on the public internet is an invitation.
const MAX_NAME: usize = 120;
const MAX_EMAIL: usize = 254;
const MAX_PHONE: usize = 40;
const MAX_REASON: usize = 2_000;
const MAX_CONTACTS: usize = 50;
const MAX_WATCHES: usize = 50;

/// Normalise an address the way we store it: trimmed and lower-cased.
///
/// This is deliberately a check on *shape*, not an attempt to decide which
/// addresses are real. The only honest test of an address is sending mail to
/// it, which is exactly what the confirmation step does.
pub fn normalise_email(raw: &str) -> Option<String> {
    let email = raw
        .trim()
        .trim_matches(|c: char| c == '<' || c == '>')
        .to_lowercase();

    if email.len() < 3 || email.len() > MAX_EMAIL {
        return None;
    }
    if email.chars().any(char::is_whitespace) {
        return None;
    }

    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    // Exactly one '@', a dot in the domain, and no empty domain labels.
    if domain.contains('@')
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
    {
        return None;
    }

    Some(email)
}

/// Trim, drop if empty, and refuse to store more than `max` characters.
fn trimmed(raw: &str, max: usize) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.chars().take(max).collect())
}

/// Split one watch field on the separators people actually paste.
fn split_addresses(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(['\n', '\r', ',', ';', ' ', '\t'])
        .filter(|part| !part.trim().is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContact {
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub relationship: String,
    pub tell_them: bool,
}

/// A validated submission, ready to be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub reason: Option<String>,
    pub discoverable: bool,
    pub source: Option<String>,
    pub contacts: Vec<NewContact>,
    pub watches: Vec<String>,
}

/// One contact row exactly as it arrived.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawContact {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub relationship: String,
    pub tell: bool,
}

/// The form as typed, before any judgement is passed on it.
///
/// Kept separate from [`Submission`] for one reason: when validation fails we
/// re-render the page with these values back in the fields. Handing somebody an
/// error and an empty form is a way of telling them their time is worth less
/// than ours.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawForm {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub reason: String,
    pub discoverable: bool,
    pub consent: bool,
    pub via: String,
    pub honeypot: String,
    pub contacts: Vec<RawContact>,
    pub watches: Vec<String>,
}

/// How many empty rows the form offers before anyone touches it.
pub const BLANK_CONTACT_ROWS: usize = 3;
pub const BLANK_WATCH_ROWS: usize = 3;

impl RawForm {
    /// An untouched form.
    pub fn blank(via: Option<String>) -> Self {
        Self {
            via: via.unwrap_or_default(),
            contacts: vec![RawContact::default(); BLANK_CONTACT_ROWS],
            watches: vec![String::new(); BLANK_WATCH_ROWS],
            ..Self::default()
        }
    }

    /// Parse an `application/x-www-form-urlencoded` body.
    ///
    /// Repeated keys build the contact and watch lists, which is why this is
    /// hand-rolled: the usual form extractors collapse duplicate keys and would
    /// silently drop every contact but the last.
    pub fn from_body(body: &str) -> Self {
        let mut form = Self::default();

        let mut names: Vec<String> = Vec::new();
        let mut emails: Vec<String> = Vec::new();
        let mut phones: Vec<String> = Vec::new();
        let mut relationships: Vec<String> = Vec::new();
        // A checkbox only submits when it is ticked, so each one carries its own
        // row index as its value. That keeps the columns lined up no matter how
        // many are ticked.
        let mut tell_rows: Vec<usize> = Vec::new();

        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            match key.as_ref() {
                "name" => form.name = value.into_owned(),
                "email" => form.email = value.into_owned(),
                "phone" => form.phone = value.into_owned(),
                "reason" => form.reason = value.into_owned(),
                "discoverable" => form.discoverable = true,
                "consent" => form.consent = true,
                "via" => form.via = value.into_owned(),
                // Never shown to a human; only a bot fills this in.
                "website" => form.honeypot = value.into_owned(),
                "contact_name" => names.push(value.into_owned()),
                "contact_email" => emails.push(value.into_owned()),
                "contact_phone" => phones.push(value.into_owned()),
                "contact_relationship" => relationships.push(value.into_owned()),
                "contact_tell" => {
                    if let Ok(index) = value.trim().parse::<usize>() {
                        tell_rows.push(index);
                    }
                }
                "watch_email" => form.watches.push(value.into_owned()),
                _ => {}
            }
        }

        let rows = names
            .len()
            .max(emails.len())
            .max(phones.len())
            .max(relationships.len())
            .min(MAX_CONTACTS);

        let at = |list: &[String], index: usize| list.get(index).cloned().unwrap_or_default();

        form.contacts = (0..rows)
            .map(|index| RawContact {
                name: at(&names, index),
                email: at(&emails, index),
                phone: at(&phones, index),
                relationship: at(&relationships, index),
                tell: tell_rows.contains(&index),
            })
            .collect();

        form
    }

    /// Whether the optional sections have anything in them. The page keeps
    /// those sections folded away until they do, so that a form somebody
    /// bounced off does not hide the half of it they had already filled in.
    pub fn has_contacts(&self) -> bool {
        self.contacts
            .iter()
            .any(|row| !row.name.trim().is_empty() || !row.email.trim().is_empty())
    }

    pub fn has_watches(&self) -> bool {
        self.watches.iter().any(|watch| !watch.trim().is_empty())
    }

    /// Pad the form back out so the re-rendered page still offers spare rows.
    pub fn padded(mut self) -> Self {
        while self.contacts.len() < BLANK_CONTACT_ROWS {
            self.contacts.push(RawContact::default());
        }
        while self.watches.len() < BLANK_WATCH_ROWS {
            self.watches.push(String::new());
        }
        self
    }

    /// Turn the form into something we are willing to store, or into the one
    /// sentence we would say to the person who filled it in.
    pub fn validate(&self) -> AppResult<Submission> {
        if !self.honeypot.trim().is_empty() {
            return Err(AppError::Invalid(
                "That submission looked automated. If you are a person, please try again."
                    .to_owned(),
            ));
        }

        let name = trimmed(&self.name, MAX_NAME)
            .ok_or_else(|| AppError::Invalid("Please tell us what to call you.".to_owned()))?;

        let email = normalise_email(&self.email).ok_or_else(|| {
            AppError::Invalid(
                "That email address does not look right. Please check it and try again.".to_owned(),
            )
        })?;

        if !self.consent {
            return Err(AppError::Invalid(
                "We need your agreement to store these details before we can put you on the list."
                    .to_owned(),
            ));
        }

        // ---- contacts ----------------------------------------------------
        let mut contacts = Vec::new();

        for (index, row) in self.contacts.iter().enumerate() {
            let Some(contact_name) = trimmed(&row.name, MAX_NAME) else {
                continue; // A blank row is somebody who changed their mind.
            };

            let contact_email = if row.email.trim().is_empty() {
                None
            } else {
                match normalise_email(&row.email) {
                    Some(valid) => Some(valid),
                    None => {
                        return Err(AppError::Invalid(format!(
                            "The email address you gave for {contact_name} does not look right."
                        )));
                    }
                }
            };

            if row.tell && contact_email.is_none() {
                return Err(AppError::Invalid(format!(
                    "You asked us to tell {contact_name} that you signed up, but we have no email \
                     address for them. Add one, or untick the box."
                )));
            }

            let _ = index;

            contacts.push(NewContact {
                name: contact_name,
                email: contact_email,
                phone: trimmed(&row.phone, MAX_PHONE),
                relationship: trimmed(&row.relationship, 40).unwrap_or_else(|| "other".to_owned()),
                tell_them: row.tell,
            });

            if contacts.len() >= MAX_CONTACTS {
                break;
            }
        }

        // ---- watches -----------------------------------------------------
        let mut watches: Vec<String> = Vec::new();
        'fields: for field in &self.watches {
            for candidate in split_addresses(field) {
                let Some(watch) = normalise_email(candidate) else {
                    return Err(AppError::Invalid(format!(
                        "\"{}\" does not look like an email address.",
                        candidate.trim().chars().take(60).collect::<String>()
                    )));
                };
                // Watching yourself is a no-op, not a mistake worth a lecture.
                if watch != email && !watches.contains(&watch) {
                    watches.push(watch);
                }
                if watches.len() >= MAX_WATCHES {
                    break 'fields;
                }
            }
        }

        Ok(Submission {
            name,
            email,
            phone: trimmed(&self.phone, MAX_PHONE),
            reason: trimmed(&self.reason, MAX_REASON),
            discoverable: self.discoverable,
            source: trimmed(&self.via, 60),
            contacts,
            watches,
        })
    }
}

fn new_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What happened. The caller shows the same page either way — whether an
/// address is already on the list is nobody's business but the owner's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Created,
    AlreadyPending,
    AlreadyConfirmed,
}

/// Store a submission. Everything is written in one transaction, including the
/// mail we intend to send, so a crash halfway through leaves no half-signed-up
/// person and no unexplained email.
pub async fn record(pool: &PgPool, submission: &Submission, base_url: &str) -> AppResult<Outcome> {
    let mut tx = pool.begin().await?;

    let existing: Option<(Uuid, Option<time::OffsetDateTime>, String)> = sqlx::query_as(
        "select id, confirmed_at, confirm_token from signup where email = $1 for update",
    )
    .bind(&submission.email)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some((signup_id, confirmed_at, token)) = existing {
        // Somebody re-submitted the form. Do not create a second row, do not
        // overwrite what they told us the first time, and do not announce on
        // the page that this address is known — only the mailbox owner learns
        // anything, and only by receiving mail.
        let outcome = if confirmed_at.is_some() {
            outbox::enqueue(
                &mut tx,
                outbox::Message::already_confirmed(signup_id, &submission.email, &submission.name),
            )
            .await?;
            Outcome::AlreadyConfirmed
        } else {
            outbox::enqueue(
                &mut tx,
                outbox::Message::confirm(
                    signup_id,
                    &submission.email,
                    &submission.name,
                    base_url,
                    &token,
                ),
            )
            .await?;
            Outcome::AlreadyPending
        };

        tx.commit().await?;
        return Ok(outcome);
    }

    let token = new_token();

    let signup_id: Uuid = sqlx::query(
        "insert into signup (name, email, phone, reason, discoverable, source, confirm_token)
         values ($1, $2, $3, $4, $5, $6, $7)
         returning id",
    )
    .bind(&submission.name)
    .bind(&submission.email)
    .bind(&submission.phone)
    .bind(&submission.reason)
    .bind(submission.discoverable)
    .bind(&submission.source)
    .bind(&token)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    for contact in &submission.contacts {
        sqlx::query(
            "insert into contact (signup_id, name, email, phone, relationship, tell_them)
             values ($1, $2, $3, $4, coalesce((select key from relationship_kind where key = $5), 'other'), $6)",
        )
        .bind(signup_id)
        .bind(&contact.name)
        .bind(&contact.email)
        .bind(&contact.phone)
        .bind(&contact.relationship)
        .bind(contact.tell_them)
        .execute(&mut *tx)
        .await?;
    }

    for watch in &submission.watches {
        sqlx::query(
            "insert into email_watch (signup_id, email) values ($1, $2)
             on conflict (signup_id, email) do nothing",
        )
        .bind(signup_id)
        .bind(watch)
        .execute(&mut *tx)
        .await?;
    }

    // Nothing else goes out yet. No invite is sent and no watch resolves until
    // this address has been confirmed — otherwise signing a stranger up would
    // be a way of making us mail their friends for you.
    outbox::enqueue(
        &mut tx,
        outbox::Message::confirm(
            signup_id,
            &submission.email,
            &submission.name,
            base_url,
            &token,
        ),
    )
    .await?;

    tx.commit().await?;
    Ok(Outcome::Created)
}

/// Mark an address confirmed and release everything that was waiting on it.
///
/// Returns the person's name, or `None` if the token means nothing — which is
/// also what an already-used token looks like from the outside.
pub async fn confirm(pool: &PgPool, token: &str) -> AppResult<Option<String>> {
    let mut tx = pool.begin().await?;

    let row: Option<(Uuid, String, String, bool, Option<time::OffsetDateTime>)> = sqlx::query_as(
        "select id, name, email, discoverable, confirmed_at from signup where confirm_token = $1 for update",
    )
    .bind(token)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((signup_id, name, email, discoverable, confirmed_at)) = row else {
        return Ok(None);
    };

    if confirmed_at.is_some() {
        // Idempotent: following the link twice is not an error.
        return Ok(Some(name));
    }

    sqlx::query("update signup set confirmed_at = now() where id = $1")
        .bind(signup_id)
        .execute(&mut *tx)
        .await?;

    release_watches(&mut tx, signup_id, &name, &email, discoverable).await?;
    send_invites(&mut tx, signup_id, &name).await?;

    tx.commit().await?;
    Ok(Some(name))
}

/// Resolve watches in both directions, and only ever between two people who
/// each said yes: the watched person must be confirmed *and* discoverable.
async fn release_watches(
    tx: &mut Transaction<'_, Postgres>,
    signup_id: Uuid,
    name: &str,
    email: &str,
    discoverable: bool,
) -> AppResult<()> {
    // 1. Watches this person placed, on people who are already here.
    let matches: Vec<(Uuid, String, String)> = sqlx::query_as(
        "update email_watch w
            set matched_at = now(), matched_signup_id = s.id
           from signup s
          where w.signup_id = $1
            and w.matched_at is null
            and s.email = w.email
            and s.confirmed_at is not null
            and s.discoverable
      returning w.id, s.name, s.email",
    )
    .bind(signup_id)
    .fetch_all(&mut **tx)
    .await?;

    for (_, matched_name, matched_email) in matches {
        outbox::enqueue(
            tx,
            outbox::Message::watch_match(signup_id, email, name, &matched_name, &matched_email),
        )
        .await?;
    }

    // 2. Watches other people placed on this address — only if this person
    //    agreed to be findable. Without that tick, the watches simply stay
    //    unresolved and nobody is told anything.
    if discoverable {
        let watchers: Vec<(Uuid, String, String)> = sqlx::query_as(
            "update email_watch w
                set matched_at = now(), matched_signup_id = $1
               from signup watcher
              where w.email = $2
                and w.matched_at is null
                and w.signup_id = watcher.id
                and watcher.confirmed_at is not null
          returning watcher.id, watcher.name, watcher.email",
        )
        .bind(signup_id)
        .bind(email)
        .fetch_all(&mut **tx)
        .await?;

        for (watcher_id, watcher_name, watcher_email) in watchers {
            outbox::enqueue(
                tx,
                outbox::Message::watch_match(
                    watcher_id,
                    &watcher_email,
                    &watcher_name,
                    name,
                    email,
                ),
            )
            .await?;
        }
    }

    Ok(())
}

/// "Inform people that you signed up."
async fn send_invites(
    tx: &mut Transaction<'_, Postgres>,
    signup_id: Uuid,
    name: &str,
) -> AppResult<()> {
    let contacts: Vec<(Uuid, String, String)> = sqlx::query_as(
        "update contact
            set told_at = now()
          where signup_id = $1 and tell_them and told_at is null and email is not null
      returning id, name, email",
    )
    .bind(signup_id)
    .fetch_all(&mut **tx)
    .await?;

    for (_, contact_name, contact_email) in contacts {
        outbox::enqueue(
            tx,
            outbox::Message::invite(signup_id, &contact_email, &contact_name, name),
        )
        .await?;
    }

    Ok(())
}

/// How many people are waiting. Used only to decide whether to show a number.
pub async fn confirmed_count(pool: &PgPool) -> AppResult<i64> {
    let count: (i64,) =
        sqlx::query_as("select count(*) from signup where confirmed_at is not null")
            .fetch_one(pool)
            .await?;
    Ok(count.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_case_and_padding() {
        assert_eq!(
            normalise_email("  Paul@Example.COM "),
            Some("paul@example.com".to_owned())
        );
        assert_eq!(normalise_email("<a@b.co>"), Some("a@b.co".to_owned()));
    }

    #[test]
    fn rejects_addresses_that_cannot_be_delivered_to() {
        for bad in [
            "",
            "paul",
            "paul@",
            "@example.com",
            "paul@example",
            "a b@example.com",
            "paul@@example.com",
            "paul@.com",
            "paul@example..com",
            "paul@example.com.",
        ] {
            assert_eq!(normalise_email(bad), None, "should have rejected {bad:?}");
        }
    }

    fn body(pairs: &[(&str, &str)]) -> String {
        form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish()
    }

    #[test]
    fn parses_a_minimal_signup() {
        let parsed = RawForm::from_body(&body(&[
            ("name", " Paul "),
            ("email", "Paul@Example.com"),
            ("consent", "1"),
        ]))
        .validate()
        .expect("should parse");

        assert_eq!(parsed.name, "Paul");
        assert_eq!(parsed.email, "paul@example.com");
        assert!(parsed.phone.is_none());
        assert!(!parsed.discoverable);
        assert!(parsed.contacts.is_empty());
        assert!(parsed.watches.is_empty());
    }

    #[test]
    fn requires_consent() {
        let error = RawForm::from_body(&body(&[("name", "Paul"), ("email", "p@example.com")]))
            .validate()
            .expect_err("should refuse");
        assert!(matches!(error, AppError::Invalid(_)));
    }

    #[test]
    fn refuses_the_honeypot() {
        let error = RawForm::from_body(&body(&[
            ("name", "Paul"),
            ("email", "p@example.com"),
            ("consent", "1"),
            ("website", "https://spam.example"),
        ]))
        .validate()
        .expect_err("should refuse");
        assert!(matches!(error, AppError::Invalid(_)));
    }

    #[test]
    fn keeps_contact_columns_aligned_when_only_some_boxes_are_ticked() {
        let parsed = RawForm::from_body(&body(&[
            ("name", "Paul"),
            ("email", "p@example.com"),
            ("consent", "1"),
            ("contact_name", "Ada"),
            ("contact_email", "ada@example.com"),
            ("contact_phone", ""),
            ("contact_relationship", "close_friend"),
            ("contact_name", "Bo"),
            ("contact_email", "bo@example.com"),
            ("contact_phone", ""),
            ("contact_relationship", "colleague"),
            ("contact_name", "Cy"),
            ("contact_email", "cy@example.com"),
            ("contact_phone", ""),
            ("contact_relationship", "family"),
            // Only the third row is ticked.
            ("contact_tell", "2"),
        ]))
        .validate()
        .expect("should parse");

        assert_eq!(parsed.contacts.len(), 3);
        assert_eq!(parsed.contacts[0].relationship, "close_friend");
        assert!(!parsed.contacts[0].tell_them);
        assert!(!parsed.contacts[1].tell_them);
        assert!(
            parsed.contacts[2].tell_them,
            "the ticked row is the one that gets told"
        );
        assert_eq!(parsed.contacts[2].name, "Cy");
    }

    #[test]
    fn drops_blank_contact_rows() {
        let parsed = RawForm::from_body(&body(&[
            ("name", "Paul"),
            ("email", "p@example.com"),
            ("consent", "1"),
            ("contact_name", "  "),
            ("contact_email", ""),
            ("contact_phone", ""),
            ("contact_relationship", "friend"),
        ]))
        .validate()
        .expect("should parse");
        assert!(parsed.contacts.is_empty());
    }

    #[test]
    fn will_not_promise_to_tell_someone_we_cannot_reach() {
        let error = RawForm::from_body(&body(&[
            ("name", "Paul"),
            ("email", "p@example.com"),
            ("consent", "1"),
            ("contact_name", "Ada"),
            ("contact_email", ""),
            ("contact_phone", ""),
            ("contact_relationship", "friend"),
            ("contact_tell", "0"),
        ]))
        .validate()
        .expect_err("should refuse");
        assert!(matches!(error, AppError::Invalid(_)));
    }

    #[test]
    fn accepts_a_pasted_list_of_watched_addresses() {
        let parsed = RawForm::from_body(&body(&[
            ("name", "Paul"),
            ("email", "p@example.com"),
            ("consent", "1"),
            (
                "watch_email",
                "Ada@example.com, bo@example.com\ncy@example.com",
            ),
            ("watch_email", "ada@example.com"),
            // Watching yourself is a no-op, not an error.
            ("watch_email", "p@example.com"),
        ]))
        .validate()
        .expect("should parse");

        assert_eq!(
            parsed.watches,
            vec!["ada@example.com", "bo@example.com", "cy@example.com"]
        );
    }

    #[test]
    fn caps_runaway_lists() {
        let mut pairs = vec![
            ("name".to_owned(), "Paul".to_owned()),
            ("email".to_owned(), "p@example.com".to_owned()),
            ("consent".to_owned(), "1".to_owned()),
        ];
        for index in 0..(MAX_WATCHES + 20) {
            pairs.push((
                "watch_email".to_owned(),
                format!("person{index}@example.com"),
            ));
        }
        let encoded = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .finish();

        let parsed = RawForm::from_body(&encoded)
            .validate()
            .expect("should parse");
        assert_eq!(parsed.watches.len(), MAX_WATCHES);
    }
}
