use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::json::{Field, field, gh};
use gitea_api::types::{Issue, IssueSearchIssuesType, Repository, StateType, User};

#[derive(Args)]
pub struct SearchCommand {
    #[command(subcommand)]
    action: SearchAction,
}

#[derive(Subcommand)]
enum SearchAction {
    /// Search repositories
    Repos(RepoSearchArgs),
    /// Search issues (across all repos)
    Issues(IssueSearchArgs),
    /// Search pull requests (across all repos)
    Prs(IssueSearchArgs),
    /// Search users
    Users(UserSearchArgs),
}

#[derive(Args)]
struct RepoSearchArgs {
    /// Search query
    query: String,

    /// Maximum results
    #[arg(short = 'L', long, default_value = "30")]
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
    #[arg(short = 'L', long, default_value = "30")]
    limit: u64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct UserSearchArgs {
    /// Search query
    query: String,

    /// Maximum results
    #[arg(short = 'L', long, default_value = "30")]
    limit: i64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

impl SearchCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            SearchAction::Repos(args) => search_repos(args).await,
            SearchAction::Issues(args) => search_issues(args, IssueSearchIssuesType::Issues).await,
            SearchAction::Prs(args) => search_issues(args, IssueSearchIssuesType::Pulls).await,
            SearchAction::Users(args) => search_users(args).await,
        }
    }
}

/// gh's `search repos --json` fields (REST names and shapes).
const REPO_FIELDS: &[Field<Repository>] = &[
    field("createdAt", |r| gh::time(r.created_at)),
    field("defaultBranch", |r| gh::v(r.default_branch.as_deref().unwrap_or(""))),
    field("description", |r| gh::v(r.description.as_deref().unwrap_or(""))),
    field("forksCount", |r| gh::v(r.forks_count.unwrap_or(0))),
    field("fullName", |r| gh::v(&r.full_name)),
    field("hasIssues", |r| gh::v(r.has_issues.unwrap_or(false))),
    field("hasProjects", |r| gh::v(r.has_projects.unwrap_or(false))),
    field("hasWiki", |r| gh::v(r.has_wiki.unwrap_or(false))),
    field("homepage", |r| gh::v(r.website.as_deref().unwrap_or(""))),
    field("id", |r| gh::v(r.id)),
    field("isArchived", |r| gh::v(r.archived.unwrap_or(false))),
    field("isFork", |r| gh::v(r.fork.unwrap_or(false))),
    field("isPrivate", |r| gh::v(r.private.unwrap_or(false))),
    field("language", |r| gh::v(r.language.as_deref().unwrap_or(""))),
    field("name", |r| gh::v(&r.name)),
    field("openIssuesCount", |r| gh::v(r.open_issues_count.unwrap_or(0))),
    field("owner", |r| gh::rest_user(r.owner.as_ref())),
    field("size", |r| gh::v(r.size.unwrap_or(0))),
    field("stargazersCount", |r| gh::v(r.stars_count.unwrap_or(0))),
    field("updatedAt", |r| gh::time(r.updated_at)),
    field("url", |r| gh::v(&r.html_url)),
    field("visibility", |r| {
        gh::v(if r.private.unwrap_or(false) {
            "private"
        } else if r.internal.unwrap_or(false) {
            "internal"
        } else {
            "public"
        })
    }),
    field("watchersCount", |r| gh::v(r.watchers_count.unwrap_or(0))),
];

async fn search_repos(args: &RepoSearchArgs) -> Result<()> {
    let json = args.json.select(REPO_FIELDS)?;
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

    if let Some(json) = json {
        return json.write_list(repos);
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

fn rest_users(us: &[User]) -> serde_json::Value {
    serde_json::Value::Array(us.iter().map(|u| gh::rest_user(Some(u))).collect())
}

/// gh's `search issues/prs --json` fields (REST names and shapes).
const ISSUE_FIELDS: &[Field<Issue>] = &[
    field("assignees", |i| rest_users(&i.assignees)),
    field("author", |i| gh::rest_user(i.user.as_ref())),
    field("body", |i| gh::v(i.body.as_deref().unwrap_or(""))),
    field("closedAt", |i| gh::time(i.closed_at)),
    field("commentsCount", |i| gh::v(i.comments.unwrap_or(0))),
    field("createdAt", |i| gh::time(i.created_at)),
    field("id", |i| gh::v(i.id)),
    field("isDraft", |i| gh::v(i.pull_request.as_ref().and_then(|p| p.draft).unwrap_or(false))),
    field("isLocked", |i| gh::v(i.is_locked.unwrap_or(false))),
    field("isPullRequest", |i| gh::v(i.pull_request.is_some())),
    field("labels", |i| gh::labels(&i.labels)),
    field("number", |i| gh::v(i.number)),
    field("repository", |i| {
        let r = i.repository.as_ref();
        gh::repo_ref(r.and_then(|r| r.name.as_deref()), r.and_then(|r| r.full_name.as_deref()))
    }),
    field("state", |i| gh::v(if matches!(i.state, Some(StateType::Closed)) { "closed" } else { "open" })),
    field("title", |i| gh::v(&i.title)),
    field("updatedAt", |i| gh::time(i.updated_at)),
    field("url", |i| gh::v(&i.html_url)),
];

async fn search_issues(args: &IssueSearchArgs, kind: IssueSearchIssuesType) -> Result<()> {
    let json = args.json.select(ISSUE_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;

    let mut req = api.issue_search_issues().q(&args.query).limit(args.limit).type_(kind);

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

    if let Some(json) = json {
        return json.write_list(&issues);
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

/// `search users --json` fields (gh has no `search users`; camelCase like
/// the rest).
const USER_FIELDS: &[Field<User>] = &[
    field("avatarUrl", |u| gh::v(&u.avatar_url)),
    field("createdAt", |u| gh::time(u.created)),
    field("description", |u| gh::v(u.description.as_deref().unwrap_or(""))),
    field("email", |u| gh::v(u.email.as_deref().unwrap_or(""))),
    field("id", |u| gh::v(u.id)),
    field("location", |u| gh::v(u.location.as_deref().unwrap_or(""))),
    field("login", |u| gh::v(&u.login)),
    field("name", |u| gh::v(u.full_name.as_deref().unwrap_or(""))),
    field("url", |u| gh::v(&u.html_url)),
    field("website", |u| gh::v(u.website.as_deref().unwrap_or(""))),
];

async fn search_users(args: &UserSearchArgs) -> Result<()> {
    let json = args.json.select(USER_FIELDS)?;
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

    if let Some(json) = json {
        return json.write_list(users);
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
