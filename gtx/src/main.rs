use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand};

mod alias;
mod api;
mod auth;
mod body;
mod browse;
mod config;
mod config_cmd;
mod gpg_key;
mod issue_meta;
mod issues;
mod json;
mod label;
mod milestone;
mod notification;
mod org;
mod package;
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
mod template;
mod variable;
mod workflow;

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GTX_GIT_SHA"),
    ", built ",
    env!("GTX_BUILD_DATE"),
    ")",
);

#[derive(Parser)]
#[command(name = "gtx", about = "Gitea CLI", version = VERSION)]
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
    /// Search repos, issues, PRs, users
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
    /// Manage packages
    Package(package::PackageCommand),
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
  gtx completion bash > ~/.local/share/bash-completion/completions/gtx

  # zsh (add fpath=(~/.zfunc $fpath) to .zshrc first)
  gtx completion zsh > ~/.zfunc/_gt

  # fish
  gtx completion fish > ~/.config/fish/completions/gtx.fish
")]
struct CompletionArgs {
    /// Shell to generate for (bash, zsh, fish, powershell, elvish)
    shell: clap_complete::Shell,
}

/// Rust ignores SIGPIPE, so `println!` into a closed pipe (`gtx ... | head -1`)
/// panics. Put back the default disposition: like gh (Go kills itself with
/// SIGPIPE on EPIPE to stdout) and other Unix tools, gtx then dies quietly
/// and the shell sees status 141. Network sockets are unaffected (std sends
/// with MSG_NOSIGNAL).
fn restore_default_sigpipe() {
    #[cfg(unix)]
    // SAFETY: called first thing in main; resetting a signal disposition to
    // its default has no memory-safety preconditions.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[tokio::main]
async fn main() {
    restore_default_sigpipe();
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
                let mut full_args = vec!["gtx".to_string()];
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
        Command::Package(cmd) => cmd.run().await,
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
                "gtx",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    };

    if let Err(ref e) = result {
        let msg = e.to_string();
        if msg.starts_with("HTTP 401") {
            eprintln!("hint: try `gtx auth login`");
        } else if msg.starts_with("HTTP 403") {
            eprintln!("hint: you don't have permission for this operation");
        }
    }

    result
}

#[cfg(test)]
mod tests {
    /// clap's own consistency checks over every subcommand: catches a
    /// shared flag (like `--template`'s `-t`) clashing with a command's own.
    #[test]
    fn cli_is_consistent() {
        <super::App as clap::CommandFactory>::command().debug_assert();
    }
}
