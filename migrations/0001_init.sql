-- wellbe.social — landing / waiting list schema.
--
-- Two rules shaped this file:
--
--   1. Nothing here exists to profile anybody. Every column is something a
--      person deliberately typed into a form, and every one of them except
--      name and email is optional.
--   2. Telling someone that another person signed up requires *both* sides to
--      have agreed. See `email_watch` below.

-- ---------------------------------------------------------------- lookup ---

-- Relationship kinds live in a table, not in a Postgres enum, so that adding
-- one is an INSERT the organisation can vote on rather than a migration the
-- engineers have to ship.
create table relationship_kind (
    key        text primary key,
    label      text        not null,
    sort_order integer     not null,
    created_at timestamptz not null default now()
);

insert into relationship_kind (key, label, sort_order) values
    ('partner',      'Partner',                   10),
    ('family',       'Family',                    20),
    ('close_friend', 'Close friend',              30),
    ('friend',       'Friend',                    40),
    ('colleague',    'Colleague',                 50),
    ('neighbour',    'Neighbour',                 60),
    ('community',    'Someone from a group I am in', 70),
    ('drifting',     'Someone I am losing touch with', 80),
    ('other',        'Something else',            90);

-- ---------------------------------------------------------------- signup ---

create table signup (
    id            uuid primary key default gen_random_uuid(),

    name          text        not null,
    -- Stored lower-cased so that uniqueness is case-insensitive without
    -- depending on the citext extension being available on the host.
    email         text        not null,
    phone         text,

    -- Optional free text: "why do you want this?"
    reason        text,

    -- Mutual-consent discovery. When false (the default) nobody is ever told
    -- that this person signed up, no matter who is watching for their address.
    discoverable  boolean     not null default false,

    -- Where they came from (?via=...). Not an ad network, just a thank-you.
    source        text,
    locale        text,

    -- Double opt-in. Nothing is sent to anyone, and no watch or invite fires,
    -- until the address has been confirmed — otherwise signing somebody else
    -- up would be a way to make us mail their friends.
    confirm_token text        not null unique,
    confirmed_at  timestamptz,

    -- "We inform you when we are ready for you."
    invited_at    timestamptz,

    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now(),

    constraint signup_email_lowercase check (email = lower(email)),
    constraint signup_email_shape     check (position('@' in email) > 1),
    constraint signup_name_not_blank  check (length(btrim(name)) > 0)
);

create unique index signup_email_key on signup (email);
create index signup_unconfirmed_idx on signup (created_at) where confirmed_at is null;
create index signup_waiting_idx     on signup (confirmed_at) where confirmed_at is not null and invited_at is null;

-- --------------------------------------------------------------- contact ---

-- "Add contact with relationship": the people you would bring with you, and
-- how you relate to them. This is the shape of the product in miniature — a
-- graph where every edge is labelled by a human, not inferred from behaviour.
create table contact (
    id           uuid primary key default gen_random_uuid(),
    signup_id    uuid        not null references signup (id) on delete cascade,

    name         text        not null,
    email        text,
    phone        text,
    relationship text        not null references relationship_kind (key),

    -- "Inform people that you signed up."
    tell_them    boolean     not null default false,
    told_at      timestamptz,

    created_at   timestamptz not null default now(),

    constraint contact_name_not_blank check (length(btrim(name)) > 0),
    constraint contact_email_lowercase check (email is null or email = lower(email)),
    -- We can only tell someone if we have somewhere to send it.
    constraint contact_tellable check (tell_them = false or email is not null)
);

create index contact_signup_idx on contact (signup_id);
create index contact_to_tell_idx on contact (signup_id) where tell_them and told_at is null;

-- ----------------------------------------------------------- email_watch ---

-- "Let me know when people with these emails sign up."
--
-- A watch on its own reveals nothing. It only ever resolves when the watched
-- person has signed up AND confirmed their address AND ticked `discoverable`.
-- Until all three are true the row simply sits here unmatched, and the watcher
-- is told nothing at all — not even that they are waiting on a stranger.
create table email_watch (
    id                uuid primary key default gen_random_uuid(),
    signup_id         uuid        not null references signup (id) on delete cascade,
    email             text        not null,

    matched_at        timestamptz,
    matched_signup_id uuid references signup (id) on delete set null,

    created_at        timestamptz not null default now(),

    unique (signup_id, email),
    constraint email_watch_lowercase check (email = lower(email)),
    constraint email_watch_shape     check (position('@' in email) > 1)
);

create index email_watch_pending_idx on email_watch (email) where matched_at is null;

-- ---------------------------------------------------------------- outbox ---

-- Every message we intend to send a human is written here in the same
-- transaction as the thing that caused it, and drained by a worker. No message
-- can be "lost between the database and the mail server", and the full list of
-- what we have ever sent anybody is one SELECT away — for us and, on request,
-- for them.
create table outbox (
    id         uuid primary key default gen_random_uuid(),
    kind       text        not null,
    signup_id  uuid references signup (id) on delete cascade,

    to_email   text,
    to_phone   text,
    payload    jsonb       not null default '{}'::jsonb,

    send_after timestamptz not null default now(),
    sent_at    timestamptz,
    attempts   integer     not null default 0,
    last_error text,

    created_at timestamptz not null default now(),

    constraint outbox_kind_known check (kind in ('confirm', 'welcome', 'invite', 'watch_match', 'ready')),
    constraint outbox_has_recipient check (to_email is not null or to_phone is not null)
);

create index outbox_pending_idx on outbox (send_after) where sent_at is null;

-- -------------------------------------------------------------- triggers ---

create or replace function touch_updated_at() returns trigger
language plpgsql as $$
begin
    new.updated_at := now();
    return new;
end;
$$;

create trigger signup_touch_updated_at
    before update on signup
    for each row execute function touch_updated_at();
