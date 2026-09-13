# wellbe.social — landing

The page people land on while wellbe.social is still being built, and the waiting list behind it.

> wellbe.social will help you connect and share with the people in your life.
> In a healthy way. On your terms.

Rust ([axum]) and Postgres ([sqlx]), server-rendered with [askama]. No JavaScript
framework, no build step, no third-party anything: one binary, one stylesheet, one
small progressive-enhancement script.

[axum]: https://docs.rs/axum
[sqlx]: https://docs.rs/sqlx
[askama]: https://docs.rs/askama

## What the page does

Everything from the brief, asked in two steps rather than one.

**Step one, on the front page.** Name, email, optional phone, optional "anything
you want us to know", and one checkbox about being findable. Short enough to
finish in a few seconds, and short enough to sit beside the slogan.

**Step two, on the page the confirmation link opens.** The people you would bring,
each with how you know them and an optional "tell them I signed up"; and the
addresses you are waiting on, which is the "let me know when these people sign up"
from the brief. None of it is required.

The second step is deliberately not on the front page. Asking somebody for their
address book before they have decided to join is the wrong order, and the page was
half again as long for it. It is also the safer order — see below.

The fourth thing from the brief, **"we inform you when we are ready for you"**, is
`wellbe-landing invite`, which takes people off the list oldest confirmed first.

## The two rules the code is built around

Both of these are easy to get quietly wrong, so they are enforced in the schema and
covered by tests rather than left to care and attention.

**Nothing leaves the building before an address is confirmed.** A signup queues one
message: the confirmation. No invite is sent and no watch resolves until somebody
follows the link, and the second step refuses to open at all for an address that has
never been confirmed. Otherwise typing a stranger's address into the form would be a
way of making us mail their friends — or, worse, a way of filling their list in for
them.

The confirmation link is therefore also the key to the second step, and it keeps
working: following it again is how somebody comes back to add more people later. The
page shows what they have already added, and adding is append-only. Re-submitting a
contact who has already been written to does nothing, which a unique index on
`(signup_id, email)` guarantees rather than hopes for.

**A watch needs consent from both sides.** "Let me know when `x@example.com` signs
up" only ever resolves if the owner of that address has signed up, confirmed, *and*
ticked "let people who already have my email address know that I joined" — which is
off by default. Until all three are true the watch simply sits there and the watcher
is told nothing, not even that they are waiting on a real person. Without that rule
the box would be a way to find out where somebody is.

See `tests/signup_flow.rs`, which exists mostly to keep these two true.

## Running it

```sh
docker compose up -d db          # Postgres on :5432
cp .env.example .env             # then: set -a; . ./.env; set +a
cargo run                        # migrations run at startup
```

Then open <http://localhost:8080>. Mail is not sent anywhere yet — the outbox worker
hands each message to a `Mailer`, and the only implementation writes to the log, so
watch the console for the confirmation links.

```sh
cargo test                       # unit tests always; integration tests need a database
TEST_DATABASE_URL=postgres://wellbe:wellbe@localhost:5432/wellbe_landing cargo test
cargo clippy --all-targets -- -D warnings
```

### Inviting people off the list

```sh
wellbe-landing invite --count 50 --dry-run   # who would be next
wellbe-landing invite --count 50             # the next fifty, oldest first
wellbe-landing invite paul@example.com       # one person
wellbe-landing invite --all
```

## Configuration

Everything has a default except the first one.

| Variable | Default | |
|---|---|---|
| `DATABASE_URL` | — | required |
| `BIND` | `0.0.0.0:8080` | |
| `BASE_URL` | `http://$BIND` | must be the address people really reach the site on; confirmation links are built from it |
| `COUNTER_THRESHOLD` | `25` | below this many confirmed signups, no counter is shown at all |
| `DRAIN_OUTBOX` | on | set `0` when a separate process drains the outbox |
| `RUST_LOG` | `wellbe_landing=info` | |

## How it is put together

```
src/
  content.rs    every word on the page, so changing what we stand for is one diff
  signup.rs     parsing, validation, the signup transaction, and the second step
  outbox.rs     outgoing mail, written in the same transaction as its cause
  invite.rs     "we are ready for you"
  web.rs        routes, headers, static files
  templates.rs  askama structs
migrations/     plain SQL, run at startup
templates/      compiled into the binary
static/         served from disk
```

A few decisions worth knowing about before you change something:

**Queries are not the `sqlx::query!` macros.** The macro variants check SQL against a
live database at compile time, which means nobody can build the project without one.
Plain `sqlx::query` with `.bind()` costs a little type safety and buys a checkout that
compiles anywhere.

**Email addresses are lower-cased in the application, not by `citext`.** One less
extension to depend on being available on whatever Postgres we end up hosted on. The
schema has a `check` constraint so a stray mixed-case insert cannot sneak past.

**Relationship kinds are a table, not a Postgres enum.** Adding one should be an
`INSERT` the organisation can decide on, not a migration the engineers have to ship.

**Mail goes through an outbox table.** Every message is written in the transaction
that caused it and drained afterwards, so nothing is ever sent for something that was
rolled back, and nothing committed can silently fail to notify anyone.

**The form works without JavaScript.** `static/app.js` adds rows on demand and moves
focus to errors. With it blocked you get three fixed rows and a working form.

**No web fonts and no analytics.** Loading a font from a third party would hand every
visitor's IP address to that third party, on the same page where we promise not to do
things like that. System fonts and a content-security-policy that only allows `self`.

## Still to do

- An SMTP implementation of `outbox::Mailer`. The trait and the whole delivery loop
  are there; there is just nowhere to send mail yet.
- A real imprint and privacy notice, which an Austrian non-profit needs and which
  should be written by the people who will be legally answerable for it.
- Rate limiting per IP on `POST /`. The honeypot and the confirmation gate handle
  casual abuse; sustained abuse needs more.
- German. The audience for a `.social` run out of Austria should not have to read
  English.
