use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tracing::info;

use inkwell::cli::migrate::{db_migrate, db_rollback, db_status};
use inkwell::config::Config;
use inkwell::db::pool::create_pool;
use inkwell::http::router::build_router;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => serve().await,
        Some("db") => match args.next().as_deref() {
            Some("migrate") => {
                let config = Config::from_env()?;
                let pool = create_pool(&config.database_url)?;
                db_migrate(&pool).await
            }
            Some("rollback") => {
                let steps = args
                    .next()
                    .as_deref()
                    .map(str::parse::<usize>)
                    .transpose()?
                    .unwrap_or(1);
                let config = Config::from_env()?;
                let pool = create_pool(&config.database_url)?;
                db_rollback(&pool, steps).await
            }
            Some("status") => {
                let config = Config::from_env()?;
                let pool = create_pool(&config.database_url)?;
                db_status(&pool).await
            }
            _ => anyhow::bail!("usage: inkwell db <migrate|rollback [n]|status>"),
        },
        _ => anyhow::bail!("usage: inkwell <serve|db migrate|db rollback [n]|db status>"),
    }
}

async fn serve() -> Result<()> {
    let config = Config::from_env()?;
    let pool = create_pool(&config.database_url)?;

    let router = build_router(Arc::new(config.clone()), pool.clone());
    let addr = SocketAddr::from((config.host.parse::<std::net::IpAddr>()?, config.port));
    let listener = TcpListener::bind(addr).await?;

    info!(host = %config.host, port = config.port, "listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            let _ = stream.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "inkwell=info,tower_http=info".into());

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .with_current_span(true)
        .init();
}
