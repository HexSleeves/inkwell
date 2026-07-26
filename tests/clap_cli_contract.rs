use clap::{CommandFactory, Parser};
use inkwell::cli::args::{
    AuthorCommand, BackupCommand, Cli, Command, DbCommand, ImportCommand, RestoreCommand,
    TokenCommand,
};
use std::path::Path;

#[test]
fn top_level_cli_parses_nested_subcommands() {
    let cli = Cli::parse_from(["inkwell", "db", "rollback", "3"]);
    assert!(matches!(
        cli.command,
        Command::Db {
            command: DbCommand::Rollback { steps: 3 }
        }
    ));

    let cli = Cli::parse_from([
        "inkwell", "author", "new", "Hello", "--slug", "hello", "--tag", "rust", "--tag", "notes",
        "--output", "hello.md", "--force",
    ]);
    assert!(matches!(
        cli.command,
        Command::Author {
            command: AuthorCommand::New { ref title, ref slug, ref tags, ref output, force, .. }
        } if title == "Hello"
            && slug.as_deref() == Some("hello")
            && tags == &vec!["rust".to_string(), "notes".to_string()]
            && output.as_deref() == Some(std::path::Path::new("hello.md"))
            && force
    ));
}

#[test]
fn clap_rejects_missing_values_and_extra_positionals() {
    assert!(Cli::try_parse_from(["inkwell", "import", "--server"]).is_err());
    assert!(Cli::try_parse_from(["inkwell", "seed", "a", "b"]).is_err());
    assert!(Cli::try_parse_from(["inkwell", "author", "publish"]).is_err());
}

#[test]
fn help_lists_the_real_command_tree() {
    let mut command = Cli::command();
    let help = command.render_long_help().to_string();
    assert!(help.contains("serve"));
    assert!(help.contains("author"));
    assert!(help.contains("import"));
    assert!(help.contains("mcp"));
    assert!(help.contains("backup"));
    assert!(help.contains("restore"));
}

/// `inkwell backup` defaults to a generated filename; `--out -` means stdout.
#[test]
fn backup_out_is_optional_and_accepts_stdout() {
    assert!(matches!(
        Cli::parse_from(["inkwell", "backup"]).command,
        Command::Backup(BackupCommand { out: None })
    ));
    assert!(matches!(
        Cli::parse_from(["inkwell", "backup", "--out", "-"]).command,
        Command::Backup(BackupCommand { ref out }) if out.as_deref() == Some(Path::new("-"))
    ));
    assert!(matches!(
        Cli::parse_from(["inkwell", "backup", "-o", "/tmp/bundle.inkwell.gz"]).command,
        Command::Backup(BackupCommand { ref out })
            if out.as_deref() == Some(Path::new("/tmp/bundle.inkwell.gz"))
    ));
}

/// `inkwell restore` requires a bundle and defaults `--overwrite` off, so the
/// destructive path is never the one you get by forgetting a flag.
#[test]
fn restore_requires_a_bundle_and_defaults_to_not_overwriting() {
    assert!(matches!(
        Cli::parse_from(["inkwell", "restore", "bundle.inkwell.gz"]).command,
        Command::Restore(RestoreCommand { ref bundle, overwrite: false })
            if bundle == Path::new("bundle.inkwell.gz")
    ));
    assert!(matches!(
        Cli::parse_from(["inkwell", "restore", "-", "--overwrite"]).command,
        Command::Restore(RestoreCommand { ref bundle, overwrite: true })
            if bundle == Path::new("-")
    ));
    assert!(
        Cli::try_parse_from(["inkwell", "restore"]).is_err(),
        "restore must require a bundle path"
    );
}

#[test]
fn author_token_subcommands_parse() {
    // create: comma-split scopes, --name, optional --server.
    let cli = Cli::parse_from([
        "inkwell",
        "author",
        "token",
        "create",
        "--name",
        "Ada",
        "--scopes",
        "write,publish",
        "--server",
        "https://blog.example.com",
    ]);
    assert!(matches!(
        cli.command,
        Command::Author {
            command: AuthorCommand::Token {
                command: TokenCommand::Create { ref name, ref scopes, ref server },
            }
        } if name == "Ada"
            && scopes == &vec!["write".to_string(), "publish".to_string()]
            && server.as_deref() == Some("https://blog.example.com")
    ));

    // revoke: positional prefix.
    let cli = Cli::parse_from(["inkwell", "author", "token", "revoke", "abc123"]);
    assert!(matches!(
        cli.command,
        Command::Author {
            command: AuthorCommand::Token {
                command: TokenCommand::Revoke { ref prefix, server: None },
            }
        } if prefix == "abc123"
    ));

    // create requires --name and at least one --scopes value.
    assert!(
        Cli::try_parse_from(["inkwell", "author", "token", "create", "--name", "Ada"]).is_err()
    );
    assert!(
        Cli::try_parse_from(["inkwell", "author", "token", "create", "--scopes", "write"]).is_err()
    );

    // list: --all flag is optional (default false), --server also optional.
    let cli = Cli::parse_from(["inkwell", "author", "token", "list", "--all"]);
    assert!(matches!(
        cli.command,
        Command::Author {
            command: AuthorCommand::Token {
                command: TokenCommand::List {
                    all: true,
                    server: None
                },
            }
        }
    ));

    let cli = Cli::parse_from(["inkwell", "author", "token", "list"]);
    assert!(matches!(
        cli.command,
        Command::Author {
            command: AuthorCommand::Token {
                command: TokenCommand::List {
                    all: false,
                    server: None
                },
            }
        }
    ));

    // prune: no required args; optional --server.
    let cli = Cli::parse_from(["inkwell", "author", "token", "prune"]);
    assert!(matches!(
        cli.command,
        Command::Author {
            command: AuthorCommand::Token {
                command: TokenCommand::Prune { server: None },
            }
        }
    ));

    let cli = Cli::parse_from([
        "inkwell",
        "author",
        "token",
        "prune",
        "--server",
        "https://blog.example.com",
    ]);
    assert!(matches!(
        cli.command,
        Command::Author {
            command: AuthorCommand::Token {
                command: TokenCommand::Prune { ref server },
            }
        } if server.as_deref() == Some("https://blog.example.com")
    ));
}

#[test]
fn import_defaults_are_preserved() {
    let cli = Cli::parse_from(["inkwell", "import", "vault", "--dry-run"]);
    assert!(matches!(
        cli.command,
        Command::Import(ImportCommand { ref vault, server: None, dry_run: true })
            if vault == std::path::Path::new("vault")
    ));
}
