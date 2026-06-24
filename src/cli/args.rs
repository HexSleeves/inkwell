use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "inkwell",
    version,
    about = "Open, API-first Markdown publishing platform"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Serve,
    Mcp,
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
    Seed(SeedCommand),
    Author {
        #[command(subcommand)]
        command: AuthorCommand,
    },
    Import(ImportCommand),
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    Migrate,
    Rollback {
        #[arg(default_value_t = 1)]
        steps: usize,
    },
    Status,
}

#[derive(Debug, Args)]
pub struct SeedCommand {
    pub vault: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum AuthorCommand {
    New {
        title: String,
        #[arg(long)]
        slug: Option<String>,
        #[arg(long, default_value = "draft")]
        status: String,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    Push {
        file: PathBuf,
        #[arg(long)]
        server: Option<String>,
    },
    Publish {
        slug: String,
        #[arg(long)]
        server: Option<String>,
    },
    Unpublish {
        slug: String,
        #[arg(long)]
        server: Option<String>,
    },
    /// Manage scoped author tokens (admin only; uses INKWELL_API_KEY).
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    /// Mint a scoped token for an author (created on first use). The token is
    /// printed ONCE and cannot be recovered afterwards.
    Create {
        #[arg(long)]
        name: String,
        /// Comma-separated scopes: read, write, publish, admin.
        #[arg(long, value_delimiter = ',', required = true)]
        scopes: Vec<String>,
        #[arg(long)]
        server: Option<String>,
    },
    /// List existing tokens (metadata only; secrets are never shown).
    List {
        #[arg(long)]
        server: Option<String>,
    },
    /// Revoke a token by its public prefix. Effective immediately.
    Revoke {
        prefix: String,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct ImportCommand {
    pub vault: PathBuf,
    #[arg(long)]
    pub server: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
}
