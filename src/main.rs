use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tracing::info;

use inkwell::cli::author;
use inkwell::cli::import;
use inkwell::cli::migrate::{db_migrate, db_rollback, db_status};
use inkwell::cli::seed;
use inkwell::config::{AuthorConfig, Config};
use inkwell::db::pool::create_pool;
use inkwell::http::router::build_router;
use inkwell::mcp;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    // The MCP server speaks JSON-RPC over stdout, so its logs must go to stderr
    // to avoid corrupting the protocol stream. Every other command logs to
    // stdout as before.
    let mcp_mode = command.as_deref() == Some("mcp");
    init_tracing(mcp_mode);

    match command.as_deref() {
        Some("serve") => serve().await,
        Some("mcp") => run_mcp().await,
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
        Some("seed") => {
            let config = Config::from_env()?;
            let pool = create_pool(&config.database_url)?;
            seed::run(&pool, args).await
        }
        Some("author") => author::run(args).await,
        Some("import") => import::run(args).await,
        _ => anyhow::bail!(
            "usage: inkwell <serve|mcp|db migrate|db rollback [n]|db status|seed [<vault>]|author <new|push|publish|unpublish>|import <vault> [--server <url>] [--dry-run]>"
        ),
    }
}

/// Run the MCP server over stdio. It is a thin HTTP client: it authenticates
/// with `INKWELL_MCP_KEY` and talks to a running inkwell server at the resolved
/// base URL (`INKWELL_API_URL`, else `HOST`/`PORT`). No database connection.
async fn run_mcp() -> Result<()> {
    let config = AuthorConfig::from_env()?;
    let base_url = config.resolve_base_url(None);
    let mcp_key = config.mcp_key.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "INKWELL_MCP_KEY is not set; the MCP server requires its own API key (separate from INKWELL_API_KEY)."
        )
    })?;
    mcp::run_stdio(base_url, mcp_key).await
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

fn init_tracing(mcp_mode: bool) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "inkwell=info,tower_http=info".into());

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .with_current_span(true);

    // The MCP server owns stdout for its JSON-RPC stream; send logs to stderr so
    // they can't corrupt the protocol. All other commands keep logging to stdout.
    if mcp_mode {
        builder.with_writer(std::io::stderr).init();
    } else {
        builder.init();
    }
}
