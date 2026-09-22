use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand};

mod alias;
mod api;
mod auth;
mod body;
mod browse;
mod config;
mod config_cmd;
mod gpg_key;
mod issues;
mod json;
mod label;
mod milestone;
mod notification;
mod org;
mod paginate;
mod project;
mod prompt;
mod pulls;
mod release;
mod repo;
mod repo_cmd;
mod run;
mod runner;
mod search;
mod secret;
mod ssh_key;
mod status;
mod variable;
mod workflow;

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GT_GIT_SHA"),
    ", built ",
    env!("GT_BUILD_DATE"),
    ")",
);

#[derive(Parser)]
#[command(name = "gt", about = "Gitea CLI", version = VERSION)]
struct App {
    /// Print HTTP request/response transcripts on stderr. Repeat for more
    /// detail (`-vv` includes request/response bodies). Tokens are masked
    /// unless `--show-secrets` is passed.
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count, global = true)]
    verbose: u8,

    /// With `-v`, print Authorization/Cookie header values unmasked. Off
    /// by default — paste-into-chat safety.
    #[arg(long = "show-secrets", global = true)]
    show_secrets: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage issues
    Issue(issues::IssueCommand),
    /// Manage pull requests
    Pr(pulls::PrCommand),
    /// Manage repositories
    Repo(repo_cmd::RepoCommand),
    /// Manage labels
    Label(label::LabelCommand),
    /// Manage milestones
    Milestone(milestone::MilestoneCommand),
    /// Manage releases
    Release(release::ReleaseCommand),
    /// Manage projects
    Project(project::ProjectCommand),
    /// Manage Actions workflow runs
    Run(run::RunCommand),
    /// Manage Actions runners
    Runner(runner::RunnerCommand),
    /// Search repos, issues, users
    Search(search::SearchCommand),
    /// Manage repository secrets
    Secret(secret::SecretCommand),
    /// Manage repository variables
    Variable(variable::VariableCommand),
    /// Manage notifications
    Notification(notification::NotificationCommand),
    /// Manage Actions workflows
    Workflow(workflow::WorkflowCommand),
    /// Manage organizations
    Org(org::OrgCommand),
    /// Manage your SSH keys
    SshKey(ssh_key::SshKeyCommand),
    /// Manage your GPG keys
    GpgKey(gpg_key::GpgKeyCommand),
    /// Manage command aliases
    Alias(alias::AliasCommand),
    /// Show status dashboard (notifications, assigned, review requests)
    Status(status::StatusCommand),
    /// Authentication commands
    Auth(auth::AuthCommand),
    /// Manage configuration
    Config(config_cmd::ConfigCommand),
    /// Open in browser
    Browse(browse::BrowseCommand),
    /// Make an authenticated API request
    Api(api::ApiCommand),
    /// Generate shell completions
    Completion(CompletionArgs),
}

#[derive(Args)]
#[command(after_long_help = "\
Install completions:

  # bash
  gt completion bash > ~/.local/share/bash-completion/completions/gt

  # zsh (add fpath=(~/.zfunc $fpath) to .zshrc first)
  gt completion zsh > ~/.zfunc/_gt

  # fish
  gt completion fish > ~/.config/fish/completions/gt.fish
")]
struct CompletionArgs {
    /// Shell to generate for (bash, zsh, fish, powershell, elvish)
    shell: clap_complete::Shell,
}

#[tokio::main]
async fn main() {
    color_eyre::install().ok();

    let result = async {
        // Check for alias expansion before clap parsing
        let raw_args: Vec<String> = std::env::args().collect();
        if raw_args.len() > 1 {
            let aliases = config::load_aliases();
            if let Some(expansion) = aliases.get(&raw_args[1]) {
                if let Some(shell_cmd) = expansion.strip_prefix('!') {
                    return alias::run_shell_alias(shell_cmd, &raw_args[2..]);
                }
                // Regular alias: expand and re-parse
                let expanded = alias::expand_alias(expansion, &raw_args[2..]);
                let mut full_args = vec!["gt".to_string()];
                full_args.extend(expanded);
                let app = App::parse_from(full_args);
                return run_app(app).await;
            }
        }

        let app = App::parse();
        run_app(app).await
    }
    .await;

    if let Err(e) = result {
        if std::env::var("RUST_BACKTRACE").is_ok() {
            eprintln!("{e:?}");
        } else {
            eprintln!("Error: {e}");
        }
        std::process::exit(1);
    }
}

async fn run_app(app: App) -> eyre::Result<()> {
    if app.verbose > 0 || app.show_secrets {
        gitea_api::verbose::set_config(gitea_api::verbose::VerboseConfig {
            level: app.verbose,
            show_secrets: app.show_secrets,
        });
    }

    let result = match app.command {
        Command::Issue(cmd) => cmd.run().await,
        Command::Pr(cmd) => cmd.run().await,
        Command::Repo(cmd) => cmd.run().await,
        Command::Label(cmd) => cmd.run().await,
        Command::Milestone(cmd) => cmd.run().await,
        Command::Release(cmd) => cmd.run().await,
        Command::Project(cmd) => cmd.run().await,
        Command::Run(cmd) => cmd.run().await,
        Command::Runner(cmd) => cmd.run().await,
        Command::Search(cmd) => cmd.run().await,
        Command::Secret(cmd) => cmd.run().await,
        Command::Variable(cmd) => cmd.run().await,
        Command::Notification(cmd) => cmd.run().await,
        Command::Workflow(cmd) => cmd.run().await,
        Command::Org(cmd) => cmd.run().await,
        Command::SshKey(cmd) => cmd.run().await,
        Command::GpgKey(cmd) => cmd.run().await,
        Command::Alias(cmd) => cmd.run().await,
        Command::Status(cmd) => cmd.run().await,
        Command::Auth(cmd) => cmd.run().await,
        Command::Config(cmd) => cmd.run().await,
        Command::Browse(cmd) => cmd.run().await,
        Command::Api(cmd) => cmd.run().await,
        Command::Completion(args) => {
            clap_complete::generate(
                args.shell,
                &mut App::command(),
                "gt",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    };

    if let Err(ref e) = result {
        let msg = e.to_string();
        if msg.starts_with("HTTP 401") {
            eprintln!("hint: try `gt auth login`");
        } else if msg.starts_with("HTTP 403") {
            eprintln!("hint: you don't have permission for this operation");
        }
    }

    result
}
