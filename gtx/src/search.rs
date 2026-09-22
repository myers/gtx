use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};

#[derive(Args)]
pub struct SearchCommand {
    #[command(subcommand)]
    action: SearchAction,
}

#[derive(Subcommand)]
enum SearchAction {
    /// Search repositories
    Repos(RepoSearchArgs),
    /// Search issues and pull requests (across all repos)
    Issues(IssueSearchArgs),
    /// Search users
    Users(UserSearchArgs),
}

#[derive(Args)]
struct RepoSearchArgs {
    /// Search query
    query: String,

    /// Maximum results
    #[arg(short, long, default_value = "30")]
    limit: i64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct IssueSearchArgs {
    /// Search query
    query: String,

    /// Filter by state (open, closed)
    #[arg(short, long)]
    state: Option<String>,

    /// Filter by owner/org
    #[arg(short, long)]
    owner: Option<String>,

    /// Maximum results
    #[arg(short, long, default_value = "30")]
    limit: u64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct UserSearchArgs {
    /// Search query
    query: String,

    /// Maximum results
    #[arg(short, long, default_value = "30")]
    limit: i64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

impl SearchCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            SearchAction::Repos(args) => search_repos(args).await,
            SearchAction::Issues(args) => search_issues(args).await,
            SearchAction::Users(args) => search_users(args).await,
        }
    }
}

const REPO_FIELDS: &[&str] = &[
    "id", "full_name", "description", "private", "fork", "archived",
    "stars_count", "forks_count", "html_url", "clone_url",
];

async fn search_repos(args: &RepoSearchArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let result = api
        .repo_search()
        .q(&args.query)
        .limit(args.limit)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let repos = &result.data;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, repos, REPO_FIELDS);
    }

    if repos.is_empty() {
        eprintln!("No repositories found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<35} {:<45} {:>5} {:>5}",
            "NAME", "DESCRIPTION", "★", "⑂"
        );
    }

    for repo in repos {
        let name = repo.full_name.as_deref().unwrap_or("");
        let desc = repo.description.as_deref().unwrap_or("");
        let truncated_desc = if desc.len() > 43 {
            format!("{}...", &desc[..40])
        } else {
            desc.to_string()
        };
        let stars = repo.stars_count.unwrap_or(0);
        let forks = repo.forks_count.unwrap_or(0);

        let mut flags = Vec::new();
        if repo.private.unwrap_or(false) {
            flags.push("private");
        }
        if repo.fork.unwrap_or(false) {
            flags.push("fork");
        }
        if repo.archived.unwrap_or(false) {
            flags.push("archived");
        }

        let name_display = if flags.is_empty() {
            name.to_string()
        } else {
            format!("{name} ({})", flags.join(", "))
        };

        println!(
            "{:<35} {:<45} {:>5} {:>5}",
            name_display, truncated_desc, stars, forks
        );
    }

    Ok(())
}

const ISSUE_FIELDS: &[&str] = &[
    "number", "title", "state", "body", "labels", "repository",
    "user", "created_at", "updated_at", "html_url",
];

async fn search_issues(args: &IssueSearchArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let mut req = api.issue_search_issues().q(&args.query).limit(args.limit);

    if let Some(ref state) = args.state {
        match state.as_str() {
            "open" => req = req.state(gitea_api::types::IssueSearchIssuesState::Open),
            "closed" => req = req.state(gitea_api::types::IssueSearchIssuesState::Closed),
            other => eyre::bail!("Invalid state: {other}. Use open or closed"),
        }
    }

    if let Some(ref owner) = args.owner {
        req = req.owner(owner);
    }

    let issues = req
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &issues, ISSUE_FIELDS);
    }

    if issues.is_empty() {
        eprintln!("No issues found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<30} {:<6} {:<40} {:<8} {}",
            "REPO", "#", "TITLE", "STATE", "UPDATED"
        );
    }

    for issue in &issues {
        let repo_name = issue
            .repository
            .as_ref()
            .and_then(|r| r.full_name.as_deref())
            .unwrap_or("");
        let truncated_repo = if repo_name.len() > 28 {
            format!("{}...", &repo_name[..25])
        } else {
            repo_name.to_string()
        };

        let number = issue.number.unwrap_or(0);
        let title = issue.title.as_deref().unwrap_or("");
        let truncated_title = if title.len() > 38 {
            format!("{}...", &title[..35])
        } else {
            title.to_string()
        };

        let state = issue
            .state
            .as_ref()
            .map(|s| format!("{s:?}").to_lowercase())
            .unwrap_or_default();
        let updated = issue
            .updated_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();

        println!(
            "{:<30} {:<6} {:<40} {:<8} {}",
            truncated_repo, number, truncated_title, state, updated
        );
    }

    Ok(())
}

const USER_FIELDS: &[&str] = &[
    "id", "login", "full_name", "email", "avatar_url", "description",
];

async fn search_users(args: &UserSearchArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let result = api
        .user_search()
        .q(&args.query)
        .limit(args.limit)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let users = &result.data;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, users, USER_FIELDS);
    }

    if users.is_empty() {
        eprintln!("No users found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<20} {:<30} {}", "LOGIN", "NAME", "EMAIL");
    }

    for user in users {
        let login = user.login.as_deref().unwrap_or("");
        let name = user.full_name.as_deref().unwrap_or("");
        let email = user.email.as_deref().unwrap_or("");
        println!("{:<20} {:<30} {}", login, name, email);
    }

    Ok(())
}
