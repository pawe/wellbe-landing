-- One contact per address per person.
--
-- The second step is reachable from the confirmation link again and again, by
-- design: it is how somebody adds people later. That makes re-submitting the
-- same person inevitable — a reload, a back button, or simply forgetting who
-- they had already listed. Without this, each of those would insert another row
-- with `told_at` still null, and the next pass would write to that friend a
-- second time.
--
-- Keeping the earliest row rather than the latest preserves any `told_at`
-- already recorded against it.

delete from contact
 where id in (
     select id
       from (
           select id,
                  row_number() over (
                      partition by signup_id, email order by created_at, id
                  ) as duplicate_rank
             from contact
            where email is not null
       ) ranked
      where duplicate_rank > 1
 );

create unique index contact_signup_email_key
    on contact (signup_id, email)
    where email is not null;
