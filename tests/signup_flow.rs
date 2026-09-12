//! End-to-end cover for the rule that is easiest to get quietly wrong:
//! telling somebody that another person signed up requires *both* of them to
//! have agreed. A regression here is a privacy incident, not a bug.
//!
//! Needs a Postgres to talk to. Set `TEST_DATABASE_URL` (or `DATABASE_URL`);
//! without one the tests skip rather than fail, so `cargo test` still works on
//! a laptop with nothing running.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;
use wellbe_landing::{config::Config, web};

fn database_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .filter(|url| !url.trim().is_empty())
}

async fn harness() -> Option<(PgPool, axum::Router)> {
    let url = database_url()?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("could not reach the test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations failed");

    let config = Config {
        database_url: url,
        bind: "127.0.0.1:0".parse().unwrap(),
        base_url: "http://test.local".to_owned(),
        counter_threshold: 25,
        // The worker would race the assertions; these tests read the outbox
        // directly, which is the thing we actually care about.
        drain_outbox: false,
    };

    let state = web::AppState {
        pool: pool.clone(),
        config: Arc::new(config),
    };

    Some((pool, web::router(state)))
}

/// Every run gets its own addresses so repeated runs never collide.
fn unique(local: &str) -> String {
    format!("{local}-{}@example.test", Uuid::new_v4().simple())
}

async fn post(router: &axum::Router, body: String) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();

    router.clone().oneshot(request).await.unwrap().status()
}

async fn get(router: &axum::Router, uri: &str) -> StatusCode {
    let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    router.clone().oneshot(request).await.unwrap().status()
}

async fn confirm(pool: &PgPool, router: &axum::Router, email: &str) -> StatusCode {
    let token: String = sqlx::query_scalar("select confirm_token from signup where email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("no such signup");

    get(router, &format!("/confirm/{token}")).await
}

async fn outbox_kinds(pool: &PgPool, to: &str) -> Vec<String> {
    sqlx::query_scalar("select kind from outbox where to_email = $1 order by created_at")
        .bind(to)
        .fetch_all(pool)
        .await
        .unwrap()
}

fn form(pairs: &[(&str, &str)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

#[tokio::test]
async fn nothing_leaves_the_building_before_the_address_is_confirmed() {
    let Some((pool, router)) = harness().await else {
        eprintln!("skipping: no TEST_DATABASE_URL");
        return;
    };

    let me = unique("unconfirmed");
    let friend = unique("friend");

    assert_eq!(
        post(
            &router,
            form(&[
                ("name", "Unconfirmed Person"),
                ("email", &me),
                ("consent", "1"),
                ("contact_name", "A Friend"),
                ("contact_email", &friend),
                ("contact_phone", ""),
                ("contact_relationship", "friend"),
                ("contact_tell", "0"),
            ])
        )
        .await,
        StatusCode::OK
    );

    assert_eq!(
        outbox_kinds(&pool, &me).await,
        vec!["confirm"],
        "before confirmation the only message we may send is the confirmation itself"
    );
    assert!(
        outbox_kinds(&pool, &friend).await.is_empty(),
        "signing somebody else up must not be a way to make us mail their friends"
    );

    assert_eq!(confirm(&pool, &router, &me).await, StatusCode::OK);
    assert_eq!(
        outbox_kinds(&pool, &friend).await,
        vec!["invite"],
        "once confirmed, the person they ticked gets exactly one message"
    );
}

#[tokio::test]
async fn a_watch_stays_silent_unless_the_watched_person_agreed_to_be_found() {
    let Some((pool, router)) = harness().await else {
        eprintln!("skipping: no TEST_DATABASE_URL");
        return;
    };

    let watcher = unique("watcher");
    let private = unique("private");
    let findable = unique("findable");

    // The watcher asks about two people, and confirms.
    assert_eq!(
        post(
            &router,
            form(&[
                ("name", "Watcher"),
                ("email", &watcher),
                ("consent", "1"),
                ("watch_email", &format!("{private}, {findable}")),
            ])
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(confirm(&pool, &router, &watcher).await, StatusCode::OK);

    // Someone who did NOT tick "let people find me" arrives and confirms.
    assert_eq!(
        post(
            &router,
            form(&[("name", "Private"), ("email", &private), ("consent", "1")])
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(confirm(&pool, &router, &private).await, StatusCode::OK);

    let matched: bool =
        sqlx::query_scalar("select matched_at is not null from email_watch where email = $1")
            .bind(&private)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !matched,
        "a watch must not resolve against someone who did not opt in"
    );
    assert_eq!(
        outbox_kinds(&pool, &watcher).await,
        vec!["confirm"],
        "the watcher must not even learn that they are waiting on a real person"
    );

    // Someone who DID tick it arrives and confirms.
    assert_eq!(
        post(
            &router,
            form(&[
                ("name", "Findable"),
                ("email", &findable),
                ("consent", "1"),
                ("discoverable", "1"),
            ])
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(confirm(&pool, &router, &findable).await, StatusCode::OK);

    assert_eq!(
        outbox_kinds(&pool, &watcher).await,
        vec!["confirm", "watch_match"],
        "with consent on both sides, and only then, the watch resolves"
    );
}

#[tokio::test]
async fn watching_someone_already_here_resolves_on_your_own_confirmation() {
    let Some((pool, router)) = harness().await else {
        eprintln!("skipping: no TEST_DATABASE_URL");
        return;
    };

    let early = unique("early");
    let late = unique("late");

    // Someone findable is already on the list.
    post(
        &router,
        form(&[
            ("name", "Early"),
            ("email", &early),
            ("consent", "1"),
            ("discoverable", "1"),
        ]),
    )
    .await;
    confirm(&pool, &router, &early).await;

    // Now somebody joins who was waiting for them.
    post(
        &router,
        form(&[
            ("name", "Late"),
            ("email", &late),
            ("consent", "1"),
            ("watch_email", &early),
        ]),
    )
    .await;
    assert_eq!(confirm(&pool, &router, &late).await, StatusCode::OK);

    assert_eq!(
        outbox_kinds(&pool, &late).await,
        vec!["confirm", "watch_match"],
        "the match must work in both directions, not only for whoever arrives last"
    );
}

#[tokio::test]
async fn filling_the_form_in_twice_does_not_create_a_second_person() {
    let Some((pool, router)) = harness().await else {
        eprintln!("skipping: no TEST_DATABASE_URL");
        return;
    };

    let email = unique("twice");
    let fields = form(&[("name", "Twice"), ("email", &email), ("consent", "1")]);

    assert_eq!(post(&router, fields.clone()).await, StatusCode::OK);
    assert_eq!(
        post(&router, fields).await,
        StatusCode::OK,
        "the second attempt must look exactly like the first from the outside"
    );

    let count: i64 = sqlx::query_scalar("select count(*) from signup where email = $1")
        .bind(&email)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn a_bounced_form_comes_back_with_what_was_typed_in_it() {
    let Some((_pool, router)) = harness().await else {
        eprintln!("skipping: no TEST_DATABASE_URL");
        return;
    };

    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form(&[
            ("name", "Someone Patient"),
            ("email", "this is not an address"),
            ("consent", "1"),
        ])))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let html = String::from_utf8_lossy(&bytes);

    assert!(
        html.contains("does not look right"),
        "it should say what went wrong"
    );
    assert!(
        html.contains("value=\"Someone Patient\""),
        "and it should not make them type their name again"
    );
}

#[tokio::test]
async fn a_meaningless_confirmation_link_is_a_plain_404() {
    let Some((_pool, router)) = harness().await else {
        eprintln!("skipping: no TEST_DATABASE_URL");
        return;
    };

    assert_eq!(
        get(&router, "/confirm/not-a-real-token").await,
        StatusCode::NOT_FOUND
    );
}
