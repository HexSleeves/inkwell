use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tracing::info;

use clap::Parser;
use inkwell::cli::args::{BackupCommand, Cli, Command, DbCommand, RestoreCommand};
use inkwell::cli::author;
use inkwell::cli::import;
use inkwell::cli::migrate::{db_migrate, db_reindex_embeddings, db_rollback, db_status};
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
            DbCommand::ReindexEmbeddings => {
                let config = Arc::new(Config::from_env()?);
                let pool = create_pool(&config.database_url)?;
                // Run migrations first so the schema is current.
                db_migrate(&pool).await?;
                let embedder = inkwell::ai::build_embedder(&config);
                db_reindex_embeddings(&pool, embedder.as_ref()).await
            }
        },
        Command::Seed(command) => {
            let config = Config::from_env()?;
            let pool = create_pool(&config.database_url)?;
            seed::run(&pool, command).await
        }
        Command::Author { command } => author::run(command).await,
        Command::Import(command) => import::run(command).await,
        Command::Backup(command) => backup(command).await,
        Command::Restore(command) => restore(command).await,
    }
}

/// `inkwell backup`: dump the deployment to a bundle and report what was
/// written. Progress goes to stderr so `--out -` stays a clean pipe.
async fn backup(command: BackupCommand) -> Result<()> {
    let config = Config::from_env()?;
    let pool = create_pool(&config.database_url)?;

    let destination = match command.out {
        Some(path) => Some(path),
        // An explicit default filename beats writing to stdout by accident.
        None => Some(std::path::PathBuf::from(
            inkwell::backup::create::default_bundle_name(time::OffsetDateTime::now_utc())?,
        )),
    };

    let media_store = inkwell::media::build_store(&config, &pool);
    let summary = inkwell::backup::create::run(&pool, media_store.as_ref(), destination).await?;
    let target = summary
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<stdout>".to_string());
    if summary.blobs_written < summary.manifest.blobs {
        eprintln!(
            "warning: {} media row(s) had no bytes in storage and were backed up without them",
            summary.manifest.blobs - summary.blobs_written
        );
    }
    eprintln!(
        "Backed up {} rows across {} tables and {} media blob(s) from the {} backend \
         (schema version {}, inkwell {}) to {}",
        summary.rows_written,
        summary.manifest.tables.len(),
        summary.blobs_written,
        summary.manifest.media_backend,
        summary.manifest.schema_version,
        summary.manifest.inkwell_version,
        target
    );
    Ok(())
}

/// `inkwell restore`: migrate the target, then load the bundle in one
/// transaction. Refuses a non-empty target without `--overwrite`.
async fn restore(command: RestoreCommand) -> Result<()> {
    let config = Config::from_env()?;
    let pool = create_pool(&config.database_url)?;

    let media_store = inkwell::media::build_store(&config, &pool);
    let summary = inkwell::backup::restore::run(
        &pool,
        media_store.as_ref(),
        Some(command.bundle),
        inkwell::backup::restore::RestoreOptions {
            overwrite: command.overwrite,
        },
    )
    .await?;

    for warning in &summary.warnings {
        eprintln!("warning: {warning}");
    }
    eprintln!(
        "Restored {} rows and {} media blob(s) into the {} backend \
         (removed {} superseded blob(s)) from a bundle written {} by inkwell {} \
         (schema version {})",
        summary.rows_restored,
        summary.blobs_restored,
        media_store.backend(),
        summary.blobs_removed,
        summary.manifest.created_at,
        summary.manifest.inkwell_version,
        summary.manifest.schema_version
    );
    Ok(())
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

    // `into_make_service_with_connect_info` exposes the peer `SocketAddr` to
    // handlers/middleware (via `ConnectInfo`), which the rate limiter uses to
    // bucket anonymous callers by client IP.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
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

/// Default log directive when neither `INKWELL_LOG` nor `RUST_LOG` is set.
const DEFAULT_LOG_FILTER: &str = "inkwell=info,tower_http=warn";

/// Initialise the tracing subscriber (CYP-46).
///
/// - **Level/targets**: `INKWELL_LOG` wins, else `RUST_LOG`, else
///   [`DEFAULT_LOG_FILTER`]. Both use `tracing-subscriber`'s `EnvFilter` syntax
///   (e.g. `inkwell=debug,inkwell::http::observability=info`).
/// - **Format**: newline-delimited JSON by default, one object per event, so a
///   log shipper can parse it. Set `INKWELL_LOG_FORMAT=pretty` for the
///   human-readable formatter while developing locally.
/// - **Writer**: stdout, except under `inkwell mcp`, which owns stdout for its
///   JSON-RPC stream and therefore logs to stderr.
fn init_tracing(mcp_mode: bool) {
    let directives = std::env::var("INKWELL_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| DEFAULT_LOG_FILTER.to_string());
    let filter = tracing_subscriber::EnvFilter::try_new(&directives)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER));

    // `pretty` is opt-in for local dev; production/staging keep machine-parsable
    // JSON so `request_id`, `route`, `status`, and `latency_ms` stay queryable.
    if std::env::var("INKWELL_LOG_FORMAT")
        .map(|value| value.trim().eq_ignore_ascii_case("pretty"))
        .unwrap_or(false)
    {
        let builder = tracing_subscriber::fmt().with_env_filter(filter).pretty();
        if mcp_mode {
            builder.with_writer(std::io::stderr).init();
        } else {
            builder.init();
        }
        return;
    }

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
