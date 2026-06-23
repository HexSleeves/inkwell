use clap::{CommandFactory, Parser};
use inkwell::cli::args::{AuthorCommand, Cli, Command, DbCommand, ImportCommand};

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
