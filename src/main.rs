use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use wellbe_landing::{config::Config, invite, outbox, web, web::AppState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wellbe_landing=info,tower_http=info,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer().compact())
        .init();

    let config = Config::from_env().inspect_err(|error| {
        tracing::error!(%error, "cannot start");
    })?;

    let args: Vec<String> = std::env::args().skip(1).collect();

    // Used by the container healthcheck, which has no curl to call /health
    // with. A landing page that cannot record a signup is down, so this asks
    // the database, not just whether the process is alive.
    if args.iter().any(|arg| arg == "--health-check") {
        let reachable = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(3))
            .connect(&config.database_url)
            .await
            .is_ok();

        if reachable {
            return Ok(());
        }
        eprintln!("unhealthy: cannot reach the database");
        std::process::exit(1);
    }

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;

    // Schema changes ship with the binary that needs them, so there is no
    // window in which a fresh deploy is talking to an old database.
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("migrations are up to date");

    // `wellbe-landing invite ...` does one job and exits; no arguments at all
    // means "be the website".
    if let Some(("invite", rest)) = args.split_first().map(|(head, rest)| (head.as_str(), rest)) {
        let plan = match invite::Plan::from_args(rest) {
            Ok(plan) => plan,
            Err(error) => {
                eprintln!("{error}");
                pool.close().await;
                std::process::exit(2);
            }
        };

        let invited = invite::run(&pool, &plan, &config.base_url).await?;

        let count = invited.len();
        let people = if count == 1 { "person" } else { "people" };
        if plan.dry_run {
            println!("would invite {count} {people}:");
        } else {
            println!("invited {count} {people}:");
        }
        for email in &invited {
            println!("  {email}");
        }

        // Queued messages still need a drain; do it here rather than leaving
        // them for whenever the server next starts.
        if !plan.dry_run && !invited.is_empty() {
            let sent = outbox::drain_once(&pool, &outbox::LogMailer).await?;
            let messages = if sent == 1 { "message" } else { "messages" };
            println!("{sent} {messages} handed to the mailer");
        }

        pool.close().await;
        return Ok(());
    }

    if config.drain_outbox {
        tokio::spawn(outbox::run_worker(pool.clone(), outbox::LogMailer));
        tracing::info!("outbox worker started");
    }

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address = %config.bind, base_url = %config.base_url, "wellbe.social landing is up");

    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config),
    };

    axum::serve(listener, web::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Let anything in flight finish and hand the connections back.
    pool.close().await;
    tracing::info!("stopped cleanly");

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl-c");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
