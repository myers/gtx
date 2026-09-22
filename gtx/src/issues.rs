use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::paginate;
use crate::repo;

#[derive(Args)]
pub struct IssueCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: IssueAction,
}

#[derive(Subcommand)]
enum IssueAction {
    /// List issues
    List(ListArgs),
    /// View an issue
    View(ViewArgs),
    /// Create an issue
    Create(CreateArgs),
    /// Delete an issue (admin only)
    Delete(DeleteArgs),
    /// Close an issue
    Close(CloseArgs),
    /// Reopen an issue
    Reopen(ReopenArgs),
    /// Add a comment to an issue
    Comment(CommentArgs),
    /// Edit an issue
    Edit(EditArgs),
    /// Show status of relevant issues
    Status(StatusArgs),
    /// Lock an issue (prevent non-collaborator comments)
    Lock(LockArgs),
    /// Unlock an issue
    Unlock(UnlockArgs),
    /// Pin an issue
    Pin(PinArgs),
    /// Unpin an issue
    Unpin(UnpinArgs),
}

#[derive(Args)]
struct EditArgs {
    /// Issue number
    number: i64,

    /// New title
    #[arg(short, long)]
    title: Option<String>,

    /// New body
    #[arg(short, long)]
    body: Option<String>,
}

#[derive(Args)]
struct StatusArgs {}

#[derive(Args)]
struct ListArgs {
    /// Filter by state
    #[arg(short, long, default_value = "open")]
    state: String,

    /// Maximum number of issues to show
    #[arg(short, long, default_value = "30")]
    limit: i64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct ViewArgs {
    /// Issue number
    number: i64,

    /// Show comments
    #[arg(short, long)]
    comments: bool,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct CreateArgs {
    /// Issue title (omit for interactive mode)
    #[arg(short, long)]
    title: Option<String>,

    /// Issue body
    #[arg(short, long)]
    body: Option<String>,

    /// Read body from file (local image/file refs are uploaded as attachments)
    #[arg(short = 'F', long)]
    body_file: Option<String>,

    /// Labels (comma-separated names — looked up by name)
    #[arg(short, long)]
    label: Vec<String>,

    /// Assignees (comma-separated usernames)
    #[arg(short, long)]
    assignee: Vec<String>,
}

#[derive(Args)]
struct DeleteArgs {
    /// Issue number
    number: i64,
}

#[derive(Args)]
struct CloseArgs {
    /// Issue number
    number: i64,
}

#[derive(Args)]
struct ReopenArgs {
    /// Issue number
    number: i64,
}

#[derive(Args)]
struct CommentArgs {
    /// Issue number
    number: i64,

    /// Comment body
    #[arg(short, long)]
    body: String,
}

#[derive(Args)]
struct LockArgs {
    /// Issue number
    number: i64,

    /// Lock reason (e.g., "off-topic", "too heated", "resolved", "spam")
    #[arg(short, long)]
    reason: Option<String>,
}

#[derive(Args)]
struct UnlockArgs {
    /// Issue number
    number: i64,
}

#[derive(Args)]
struct PinArgs {
    /// Issue number
    number: i64,
}

#[derive(Args)]
struct UnpinArgs {
    /// Issue number
    number: i64,
}

impl IssueCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            IssueAction::List(args) => list_issues(&self.repo, args).await,
            IssueAction::View(args) => view_issue(&self.repo, args).await,
            IssueAction::Create(args) => create_issue(&self.repo, args).await,
            IssueAction::Delete(args) => delete_issue(&self.repo, args).await,
            IssueAction::Close(args) => set_issue_state(self.repo.repo.as_deref(), args.number, "closed").await,
            IssueAction::Reopen(args) => set_issue_state(self.repo.repo.as_deref(), args.number, "open").await,
            IssueAction::Comment(args) => comment_issue(&self.repo, args).await,
            IssueAction::Edit(args) => edit_issue(&self.repo, args).await,
            IssueAction::Status(args) => status_issues(&self.repo, args).await,
            IssueAction::Lock(args) => lock_issue(&self.repo, args).await,
            IssueAction::Unlock(args) => unlock_issue(&self.repo, args).await,
            IssueAction::Pin(args) => pin_issue(&self.repo, args).await,
            IssueAction::Unpin(args) => unpin_issue(&self.repo, args).await,
        }
    }
}

const ISSUE_FIELDS: &[&str] = &[
    "number", "title", "state", "body", "labels", "assignees", "milestone",
    "comments", "created_at", "updated_at", "closed_at", "due_date", "url",
    "html_url", "user", "repository",
];

async fn list_issues(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // Validate state before paginating
    match args.state.as_str() {
        "open" | "closed" | "all" => {}
        other => eyre::bail!("Invalid state: {other}. Use open, closed, or all"),
    }

    let state_str = args.state.clone();
    let issues = paginate::paginate(args.limit, 50, |page, per_page| {
        let api = &api;
        let state_str = &state_str;
        async move {
            let mut req = api
                .issue_list_issues()
                .owner(owner)
                .repo(repo)
                .page(page)
                .limit(per_page);
            match state_str.as_str() {
                "open" => req = req.state(gitea_api::types::IssueListIssuesState::Open),
                "closed" => req = req.state(gitea_api::types::IssueListIssuesState::Closed),
                _ => {}
            }
            Ok(req.send().await.map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?.into_inner())
        }
    })
    .await?;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &issues, &ISSUE_FIELDS);
    }

    if issues.is_empty() {
        eprintln!("No issues found");
        return Ok(());
    }

    // Table output
    let is_tty = atty_check();

    // Header
    if is_tty {
        println!(
            "{:<6} {:<50} {:<20} {}",
            "#", "TITLE", "LABELS", "UPDATED"
        );
    }

    for issue in &issues {
        let number = issue.number.unwrap_or(0);
        let title = issue.title.as_deref().unwrap_or("");
        let truncated_title = if title.len() > 48 {
            format!("{}...", &title[..45])
        } else {
            title.to_string()
        };

        let labels = issue
            .labels
            .iter()
            .filter_map(|l| l.name.as_deref())
            .collect::<Vec<_>>()
            .join(",");
        let truncated_labels = if labels.len() > 18 {
            format!("{}...", &labels[..15])
        } else {
            labels
        };

        let updated = issue
            .updated_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();

        println!(
            "{:<6} {:<50} {:<20} {}",
            number, truncated_title, truncated_labels, updated
        );
    }

    Ok(())
}

async fn view_issue(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let issue = api
        .issue_get_issue()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json {
        if args.comments {
            let comments = api
                .issue_get_comments()
                .owner(owner)
                .repo(repo)
                .index(args.number)
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner();
            let combined = serde_json::json!({
                "issue": issue,
                "comments": comments,
            });
            println!("{}", serde_json::to_string_pretty(&combined)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&issue)?);
        }
        return Ok(());
    }

    // Header: title and number
    let number = issue.number.unwrap_or(0);
    let title = issue.title.as_deref().unwrap_or("(no title)");
    let state = issue
        .state
        .as_ref()
        .map(|s| format!("{s:?}"))
        .unwrap_or_default()
        .to_lowercase();

    println!("{title} #{number}");
    println!("{state} — opened by {} {}",
        issue.user.as_ref().and_then(|u| u.login.as_deref()).unwrap_or("unknown"),
        issue.created_at.map(|dt| relative_time(dt)).unwrap_or_default(),
    );

    // Labels
    if !issue.labels.is_empty() {
        let label_names: Vec<&str> = issue.labels.iter().filter_map(|l| l.name.as_deref()).collect();
        println!("Labels: {}", label_names.join(", "));
    }

    // Assignees
    if !issue.assignees.is_empty() {
        let names: Vec<&str> = issue.assignees.iter().filter_map(|u| u.login.as_deref()).collect();
        println!("Assignees: {}", names.join(", "));
    }

    // Milestone
    if let Some(ref ms) = issue.milestone {
        if let Some(ref title) = ms.title {
            println!("Milestone: {title}");
        }
    }

    // Body
    if let Some(ref body) = issue.body {
        if !body.is_empty() {
            println!();
            println!("{body}");
        }
    }

    // URL
    if let Some(ref url) = issue.html_url {
        println!();
        println!("{url}");
    }

    // Comments
    if args.comments {
        let comments = api
            .issue_get_comments()
            .owner(owner)
            .repo(repo)
            .index(args.number)
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner();

        if comments.is_empty() {
            println!("\nNo comments.");
        } else {
            println!("\n--- {} comment{} ---", comments.len(), if comments.len() == 1 { "" } else { "s" });
            for c in &comments {
                let author = c.user.as_ref().and_then(|u| u.login.as_deref()).unwrap_or("unknown");
                let when = c.created_at.map(|dt| relative_time(dt)).unwrap_or_default();
                let body = c.body.as_deref().unwrap_or("");
                println!("\n{author} ({when}):");
                println!("{body}");
            }
        }
    } else {
        let count = issue.comments.unwrap_or(0);
        if count > 0 {
            println!("\n{count} comment{} (use -c to show)", if count == 1 { "" } else { "s" });
        }
    }

    Ok(())
}

/// Collected inputs for creating an issue (from flags or interactive prompts).
struct IssueInput {
    title: String,
    body: String,
    label_ids: Vec<i64>,
    assignees: Vec<String>,
    milestone_id: Option<i64>,
}

async fn create_issue(repo_args: &repo::RepoArgs, args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // Resolve body: --body-file takes precedence over --body
    let body_text = if let Some(ref path) = args.body_file {
        Some(crate::body::read_body_file(path)?)
    } else {
        args.body.clone()
    };
    let body_file_dir = args
        .body_file
        .as_ref()
        .and_then(|p| std::path::Path::new(p).parent())
        .map(|p| p.to_path_buf());

    let input = if let Some(ref title) = args.title {
        // Non-interactive: resolve label names to IDs
        let label_ids = if args.label.is_empty() {
            vec![]
        } else {
            resolve_label_ids(&api, owner, repo, &args.label).await?
        };
        IssueInput {
            title: title.clone(),
            body: body_text.unwrap_or_default(),
            label_ids,
            assignees: args.assignee.clone(),
            milestone_id: None,
        }
    } else {
        // Interactive
        if !atty_check() {
            eyre::bail!("provide --title when not running interactively");
        }
        interactive_create_issue(&api, owner, repo).await?
    };

    let issue = api
        .issue_create_issue()
        .owner(owner)
        .repo(repo)
        .body_map(|mut b| {
            b = b
                .title(input.title.clone())
                .body(input.body.clone())
                .assignees(input.assignees.clone())
                .labels(input.label_ids.clone());
            if let Some(ms) = input.milestone_id {
                b = b.milestone(ms);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let number = issue.number.unwrap_or(0);
    let url = issue.html_url.as_deref().unwrap_or("");
    eprintln!("Created issue #{number}: {url}");

    // Upload local file attachments and rewrite body if needed
    if let Some(ref base_dir) = body_file_dir {
        let refs = crate::body::find_local_refs(&input.body, base_dir);
        if !refs.is_empty() {
            let new_body =
                crate::body::upload_and_rewrite(&api, &config, owner, repo, number, &input.body, base_dir)
                    .await?;
            // Update issue body with rewritten URLs
            api.issue_edit_issue()
                .owner(owner)
                .repo(repo)
                .index(number)
                .body_map(|b| b.body(new_body.clone()))
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;
            eprintln!("Updated body with uploaded attachments");
        }
    }

    Ok(())
}

/// Resolve label names to IDs by fetching labels from the repo.
async fn resolve_label_ids(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    names: &[String],
) -> Result<Vec<i64>> {
    let labels = api
        .issue_list_labels()
        .owner(owner)
        .repo(repo)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let mut ids = Vec::new();
    for name in names {
        let found = labels
            .iter()
            .find(|l| l.name.as_deref() == Some(name.as_str()));
        match found {
            Some(l) => ids.push(l.id.unwrap_or(0)),
            None => eyre::bail!("Label not found: {name}"),
        }
    }
    Ok(ids)
}

/// Interactive issue creation flow (gh-style).
async fn interactive_create_issue(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
) -> Result<IssueInput> {
    // 1. Title (required)
    let title = inquire::Text::new("Title:")
        .with_validator(|s: &str| {
            if s.trim().is_empty() {
                Ok(inquire::validator::Validation::Invalid("Title is required".into()))
            } else {
                Ok(inquire::validator::Validation::Valid)
            }
        })
        .prompt()?;

    // 2. Body via editor
    let body = crate::prompt::edit_body("")?;

    // 3. What's next?
    let action = inquire::Select::new("What's next?", vec!["Submit", "Add metadata", "Cancel"])
        .prompt()?;

    let mut label_ids = Vec::new();
    let mut assignees = Vec::new();
    let mut milestone_id = None;

    if action == "Cancel" {
        eyre::bail!("Cancelled");
    }

    if action == "Add metadata" {
        let choices = inquire::MultiSelect::new(
            "What would you like to add?",
            vec!["Labels", "Assignees", "Milestone"],
        )
        .prompt()?;

        if choices.contains(&"Labels") {
            let api_labels = api
                .issue_list_labels()
                .owner(owner)
                .repo(repo)
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner();

            let label_options: Vec<(String, i64)> = api_labels
                .iter()
                .filter_map(|l| Some((l.name.clone()?, l.id?)))
                .collect();

            if label_options.is_empty() {
                eprintln!("No labels found in this repository.");
            } else {
                let names: Vec<&str> = label_options.iter().map(|(n, _)| n.as_str()).collect();
                let selected = inquire::MultiSelect::new("Labels:", names).prompt()?;
                for name in selected {
                    if let Some((_, id)) = label_options.iter().find(|(n, _)| n == name) {
                        label_ids.push(*id);
                    }
                }
            }
        }

        if choices.contains(&"Assignees") {
            let input = inquire::Text::new("Assignees (comma-separated usernames):").prompt()?;
            assignees = input
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        if choices.contains(&"Milestone") {
            let api_milestones = api
                .issue_get_milestones_list()
                .owner(owner)
                .repo(repo)
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner();

            let ms_options: Vec<(String, i64)> = api_milestones
                .iter()
                .filter_map(|m| Some((m.title.clone()?, m.id?)))
                .collect();

            if ms_options.is_empty() {
                eprintln!("No milestones found in this repository.");
            } else {
                let names: Vec<&str> = ms_options.iter().map(|(n, _)| n.as_str()).collect();
                let selected = inquire::Select::new("Milestone:", names).prompt()?;
                if let Some((_, id)) = ms_options.iter().find(|(n, _)| n == selected) {
                    milestone_id = Some(*id);
                }
            }
        }

        // Confirm after metadata
        let confirm = inquire::Select::new("What's next?", vec!["Submit", "Cancel"]).prompt()?;
        if confirm == "Cancel" {
            eyre::bail!("Cancelled");
        }
    }

    Ok(IssueInput {
        title,
        body,
        label_ids,
        assignees,
        milestone_id,
    })
}

async fn delete_issue(repo_args: &repo::RepoArgs, args: &DeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.issue_delete()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Deleted issue #{}", args.number);
    Ok(())
}

async fn set_issue_state(repo_opt: Option<&str>, number: i64, state: &str) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_opt, &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.issue_edit_issue()
        .owner(owner)
        .repo(repo)
        .index(number)
        .body_map(|b| b.state(state.to_string()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Issue #{number} {state}");
    Ok(())
}

async fn comment_issue(repo_args: &repo::RepoArgs, args: &CommentArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.issue_create_comment()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .body_map(|b| b.body(args.body.clone()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Comment added to issue #{}", args.number);
    Ok(())
}

async fn edit_issue(repo_args: &repo::RepoArgs, args: &EditArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let mut builder = api.issue_edit_issue().owner(owner).repo(repo).index(args.number);

    if args.title.is_some() || args.body.is_some() {
        let title = args.title.clone();
        let body = args.body.clone();
        builder = builder.body_map(move |mut b| {
            if let Some(t) = title {
                b = b.title(t);
            }
            if let Some(bd) = body {
                b = b.body(bd);
            }
            b
        });
    }

    builder
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Issue #{} updated", args.number);
    Ok(())
}

async fn status_issues(repo_args: &repo::RepoArgs, _args: &StatusArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // Show open issues assigned to the current user
    let issues = api
        .issue_list_issues()
        .owner(owner)
        .repo(repo)
        .state(gitea_api::types::IssueListIssuesState::Open)
        .page(1)
        .limit(20)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if issues.is_empty() {
        println!("No open issues");
        return Ok(());
    }

    println!("Open issues in {}/{}:", owner, repo);
    for issue in &issues {
        let number = issue.number.unwrap_or(0);
        let title = issue.title.as_deref().unwrap_or("");
        let updated = issue
            .updated_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();
        println!("  #{number:<5} {title} ({updated})");
    }

    Ok(())
}

pub fn relative_time(dt: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(dt);

    if delta.num_minutes() < 1 {
        "just now".to_string()
    } else if delta.num_hours() < 1 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_days() < 1 {
        format!("{}h ago", delta.num_hours())
    } else if delta.num_weeks() < 1 {
        format!("{}d ago", delta.num_days())
    } else if delta.num_weeks() < 52 {
        format!("{}w ago", delta.num_weeks())
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

pub fn atty_check() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

async fn lock_issue(repo_args: &repo::RepoArgs, args: &LockArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.issue_lock_issue()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .body_map(|mut b| {
            if let Some(ref reason) = args.reason {
                b = b.lock_reason(reason.clone());
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Locked issue #{}", args.number);
    Ok(())
}

async fn unlock_issue(repo_args: &repo::RepoArgs, args: &UnlockArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.issue_unlock_issue()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Unlocked issue #{}", args.number);
    Ok(())
}

async fn pin_issue(repo_args: &repo::RepoArgs, args: &PinArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.pin_issue()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Pinned issue #{}", args.number);
    Ok(())
}

async fn unpin_issue(repo_args: &repo::RepoArgs, args: &UnpinArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.unpin_issue()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Unpinned issue #{}", args.number);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    #[test]
    fn test_relative_time_just_now() {
        let now = Utc::now();
        assert_eq!(relative_time(now), "just now");
    }

    #[test]
    fn test_relative_time_minutes() {
        let t = Utc::now() - Duration::minutes(5);
        assert_eq!(relative_time(t), "5m ago");
    }

    #[test]
    fn test_relative_time_hours() {
        let t = Utc::now() - Duration::hours(3);
        assert_eq!(relative_time(t), "3h ago");
    }

    #[test]
    fn test_relative_time_days() {
        let t = Utc::now() - Duration::days(4);
        assert_eq!(relative_time(t), "4d ago");
    }

    #[test]
    fn test_relative_time_weeks() {
        let t = Utc::now() - Duration::weeks(10);
        assert_eq!(relative_time(t), "10w ago");
    }

    #[test]
    fn test_relative_time_old_date() {
        let t = Utc::now() - Duration::weeks(60);
        assert!(relative_time(t).contains('-')); // returns YYYY-MM-DD format
    }
}
