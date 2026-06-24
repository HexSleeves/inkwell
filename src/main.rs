use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tracing::info;

use clap::Parser;
use inkwell::cli::args::{Cli, Command, DbCommand};
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
    let cli = Cli::parse();
    // The MCP server speaks JSON-RPC over stdout, so its logs must go to stderr
    // to avoid corrupting the protocol stream. Every other command logs to
    // stdout as before.
    let mcp_mode = matches!(cli.command, Command::Mcp);
    init_tracing(mcp_mode);

    match cli.command {
        Command::Serve => serve().await,
        Command::Mcp => run_mcp().await,
        Command::Db { command } => match command {
            DbCommand::Migrate => {
                let config = Config::from_env()?;
                let pool = create_pool(&config.database_url)?;
                db_migrate(&pool).await
            }
            DbCommand::Rollback { steps } => {
                let config = Config::from_env()?;
                let pool = create_pool(&config.database_url)?;
                db_rollback(&pool, steps).await
            }
            DbCommand::Status => {
                let config = Config::from_env()?;
                let pool = create_pool(&config.database_url)?;
                db_status(&pool).await
            }
        },
        Command::Seed(command) => {
            let config = Config::from_env()?;
            let pool = create_pool(&config.database_url)?;
            seed::run(&pool, command).await
        }
        Command::Author { command } => author::run(command).await,
        Command::Import(command) => import::run(command).await,
    }
}

/// Run the MCP server over stdio. It is a thin HTTP client: it authenticates
/// with `INKWELL_API_KEY` — set this to a **scoped token** (`inkwell author token
/// create`) so MCP access is independently grant/revocable — and talks to a
/// running inkwell server at the resolved base URL (`INKWELL_API_URL`, else
/// `HOST`/`PORT`). No database connection. (The separate `INKWELL_MCP_KEY` was
/// retired in slice 4.)
async fn run_mcp() -> Result<()> {
    let config = AuthorConfig::from_env()?;
    let base_url = config.resolve_base_url(None);
    let api_key = config.api_key.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "INKWELL_API_KEY is not set; the MCP server requires an API key. Set it to a scoped token minted with `inkwell author token create`."
        )
    })?;
    mcp::run_stdio(base_url, api_key).await
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
