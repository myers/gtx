use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::paginate;
use crate::repo;

#[derive(Args)]
pub struct PrCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: PrAction,
}

#[derive(Subcommand)]
enum PrAction {
    /// List pull requests
    List(ListArgs),
    /// View a pull request
    View(ViewArgs),
    /// Create a pull request
    Create(CreateArgs),
    /// Checkout a pull request branch
    Checkout(CheckoutArgs),
    /// Merge a pull request
    Merge(MergeArgs),
    /// Close a pull request
    Close(CloseArgs),
    /// Reopen a pull request
    Reopen(ReopenArgs),
    /// Add a comment to a pull request
    Comment(CommentArgs),
    /// View pull request diff
    Diff(DiffArgs),
    /// Submit a review on a pull request
    Review(ReviewArgs),
    /// Show CI status for a pull request
    Checks(ChecksArgs),
    /// Edit a pull request (title, body, labels, assignees, milestone)
    Edit(EditArgs),
    /// Update PR branch (merge base branch into head)
    UpdateBranch(UpdateBranchArgs),
}

#[derive(Args)]
struct ListArgs {
    /// Filter by state (open, closed, all)
    #[arg(short, long, default_value = "open")]
    state: String,

    /// Maximum number of PRs to show
    #[arg(short, long, default_value = "30")]
    limit: i64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct ViewArgs {
    /// Pull request number
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
    /// PR title (omit for interactive mode)
    #[arg(short, long)]
    title: Option<String>,

    /// PR body
    #[arg(short, long)]
    body: Option<String>,

    /// Read body from file (local image/file refs are uploaded as attachments)
    #[arg(short = 'F', long)]
    body_file: Option<String>,

    /// Base branch (defaults to repo default branch)
    #[arg(long, default_value = "main")]
    base: String,

    /// Head branch (defaults to current branch)
    #[arg(long)]
    head: Option<String>,
}

#[derive(Args)]
struct CheckoutArgs {
    /// PR number
    number: i64,
}

#[derive(Args)]
struct MergeArgs {
    /// PR number
    number: i64,

    /// Merge method (merge, rebase, squash)
    #[arg(short, long, default_value = "merge")]
    method: String,

    /// Delete branch after merge
    #[arg(short, long)]
    delete_branch: bool,
}

#[derive(Args)]
struct CloseArgs {
    /// PR number
    number: i64,
}

#[derive(Args)]
struct ReopenArgs {
    /// PR number
    number: i64,
}

#[derive(Args)]
struct CommentArgs {
    /// PR number
    number: i64,

    /// Comment body
    #[arg(short, long)]
    body: String,
}

#[derive(Args)]
struct ReviewArgs {
    /// PR number
    number: i64,

    /// Review action: approve, request-changes, comment
    #[arg(short, long, default_value = "approve")]
    action: String,

    /// Review body/comment
    #[arg(short, long, default_value = "")]
    body: String,
}

#[derive(Args)]
struct ChecksArgs {
    /// PR number
    number: i64,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct DiffArgs {
    /// PR number
    number: i64,
}

#[derive(Args)]
struct EditArgs {
    /// PR number
    number: i64,

    /// New title
    #[arg(short, long)]
    title: Option<String>,

    /// New body
    #[arg(short, long)]
    body: Option<String>,

    /// Add labels (comma-separated names, looked up by name)
    #[arg(short, long)]
    label: Vec<String>,

    /// Set assignees (comma-separated usernames, replaces existing)
    #[arg(short, long)]
    assignee: Vec<String>,

    /// Set milestone (by name)
    #[arg(short, long)]
    milestone: Option<String>,
}

#[derive(Args)]
struct UpdateBranchArgs {
    /// PR number
    number: i64,
}

impl PrCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            PrAction::List(args) => list_prs(&self.repo, args).await,
            PrAction::View(args) => view_pr(&self.repo, args).await,
            PrAction::Create(args) => create_pr(&self.repo, args).await,
            PrAction::Checkout(args) => checkout_pr(&self.repo, args).await,
            PrAction::Merge(args) => merge_pr(&self.repo, args).await,
            PrAction::Close(args) => set_pr_state(self.repo.repo.as_deref(), args.number, "closed").await,
            PrAction::Reopen(args) => set_pr_state(self.repo.repo.as_deref(), args.number, "open").await,
            PrAction::Comment(args) => comment_pr(&self.repo, args).await,
            PrAction::Diff(args) => diff_pr(&self.repo, args).await,
            PrAction::Review(args) => review_pr(&self.repo, args).await,
            PrAction::Checks(args) => checks_pr(&self.repo, args).await,
            PrAction::Edit(args) => edit_pr(&self.repo, args).await,
            PrAction::UpdateBranch(args) => update_branch(&self.repo, args).await,
        }
    }
}

const PR_FIELDS: &[&str] = &[
    "number", "title", "state", "body", "labels", "assignees", "milestone",
    "head", "base", "merged", "merged_at", "mergeable", "comments",
    "created_at", "updated_at", "closed_at", "url", "html_url", "user",
    "diff_url", "patch_url",
];

async fn list_prs(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
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
    let prs = paginate::paginate(args.limit, 50, |page, per_page| {
        let api = &api;
        let state_str = &state_str;
        async move {
            let mut req = api
                .repo_list_pull_requests()
                .owner(owner)
                .repo(repo)
                .page(page as u64)
                .limit(per_page);
            match state_str.as_str() {
                "open" => req = req.state(gitea_api::types::RepoListPullRequestsState::Open),
                "closed" => req = req.state(gitea_api::types::RepoListPullRequestsState::Closed),
                _ => {}
            }
            Ok(req.send().await.map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?.into_inner())
        }
    })
    .await?;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &prs, &PR_FIELDS);
    }

    if prs.is_empty() {
        eprintln!("No pull requests found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<6} {:<50} {:<15} {}",
            "#", "TITLE", "AUTHOR", "UPDATED"
        );
    }

    for pr in &prs {
        let number = pr.number.unwrap_or(0);
        let title = pr.title.as_deref().unwrap_or("");
        let truncated_title = if title.len() > 48 {
            format!("{}...", &title[..45])
        } else {
            title.to_string()
        };

        let author = pr
            .user
            .as_ref()
            .and_then(|u| u.login.as_deref())
            .unwrap_or("");
        let truncated_author = if author.len() > 13 {
            format!("{}...", &author[..10])
        } else {
            author.to_string()
        };

        let updated = pr
            .updated_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();

        println!(
            "{:<6} {:<50} {:<15} {}",
            number, truncated_title, truncated_author, updated
        );
    }

    Ok(())
}

async fn view_pr(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let pr = api
        .repo_get_pull_request()
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
                "pr": pr,
                "comments": comments,
            });
            println!("{}", serde_json::to_string_pretty(&combined)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&pr)?);
        }
        return Ok(());
    }

    let number = pr.number.unwrap_or(0);
    let title = pr.title.as_deref().unwrap_or("(no title)");
    let state = pr
        .state
        .as_ref()
        .map(|s| format!("{s:?}"))
        .unwrap_or_default()
        .to_lowercase();

    let merged = pr.merged.unwrap_or(false);
    let status = if merged { "merged".to_string() } else { state };

    println!("{title} #{number}");
    println!(
        "{status} — opened by {} {}",
        pr.user
            .as_ref()
            .and_then(|u| u.login.as_deref())
            .unwrap_or("unknown"),
        pr.created_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default(),
    );

    // Head/base branches
    if let (Some(head), Some(base)) = (
        pr.head.as_ref().and_then(|h| h.label.as_deref()),
        pr.base.as_ref().and_then(|b| b.label.as_deref()),
    ) {
        println!("{head} -> {base}");
    }

    // Labels
    if !pr.labels.is_empty() {
        let label_names: Vec<&str> = pr.labels.iter().filter_map(|l| l.name.as_deref()).collect();
        println!("Labels: {}", label_names.join(", "));
    }

    // Assignees
    if !pr.assignees.is_empty() {
        let names: Vec<&str> = pr
            .assignees
            .iter()
            .filter_map(|u| u.login.as_deref())
            .collect();
        println!("Assignees: {}", names.join(", "));
    }

    // Milestone
    if let Some(ref ms) = pr.milestone {
        if let Some(ref title) = ms.title {
            println!("Milestone: {title}");
        }
    }

    // Diff stats
    if let (Some(adds), Some(dels)) = (pr.additions, pr.deletions) {
        let files = pr.changed_files.unwrap_or(0);
        println!("+{adds} -{dels} ({files} files)");
    }

    // Body
    if let Some(ref body) = pr.body {
        if !body.is_empty() {
            println!();
            println!("{body}");
        }
    }

    // URL
    if let Some(ref url) = pr.html_url {
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
            println!(
                "\n--- {} comment{} ---",
                comments.len(),
                if comments.len() == 1 { "" } else { "s" }
            );
            for c in &comments {
                let author = c
                    .user
                    .as_ref()
                    .and_then(|u| u.login.as_deref())
                    .unwrap_or("unknown");
                let when = c
                    .created_at
                    .map(|dt| relative_time(dt))
                    .unwrap_or_default();
                let body = c.body.as_deref().unwrap_or("");
                println!("\n{author} ({when}):");
                println!("{body}");
            }
        }
    } else {
        let count = pr.comments.unwrap_or(0);
        if count > 0 {
            println!(
                "\n{count} comment{} (use -c to show)",
                if count == 1 { "" } else { "s" }
            );
        }
    }

    Ok(())
}

fn detect_current_branch() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(|_| eyre::eyre!("Failed to detect current branch"))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn create_pr(repo_args: &repo::RepoArgs, args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let head = match &args.head {
        Some(h) => h.clone(),
        None => detect_current_branch()?,
    };

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

    let (title, body) = if let Some(ref title) = args.title {
        (title.clone(), body_text.unwrap_or_default())
    } else {
        // Interactive mode
        if !atty_check() {
            eyre::bail!("provide --title when not running interactively");
        }
        interactive_create_pr(&head, &args.base)?
    };

    let pr = api
        .repo_create_pull_request()
        .owner(owner)
        .repo(repo)
        .body_map(|b| {
            b.title(title.clone())
                .body(body.clone())
                .base(args.base.clone())
                .head(head.clone())
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let number = pr.number.unwrap_or(0);
    let url = pr.html_url.as_deref().unwrap_or("");
    eprintln!("Created PR #{number}: {url}");

    // Upload local file attachments and rewrite body if needed
    if let Some(ref base_dir) = body_file_dir {
        let refs = crate::body::find_local_refs(&body, base_dir);
        if !refs.is_empty() {
            let new_body =
                crate::body::upload_and_rewrite(&api, &config, owner, repo, number, &body, base_dir)
                    .await?;
            api.repo_edit_pull_request()
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

fn interactive_create_pr(head: &str, base: &str) -> Result<(String, String)> {
    eprintln!("Creating PR: {head} → {base}\n");

    let title = inquire::Text::new("Title:")
        .with_validator(|s: &str| {
            if s.trim().is_empty() {
                Ok(inquire::validator::Validation::Invalid("Title is required".into()))
            } else {
                Ok(inquire::validator::Validation::Valid)
            }
        })
        .prompt()?;

    let body = crate::prompt::edit_body("")?;

    let action = inquire::Select::new("What's next?", vec!["Submit", "Cancel"]).prompt()?;
    if action == "Cancel" {
        eyre::bail!("Cancelled");
    }

    Ok((title, body))
}

async fn checkout_pr(repo_args: &repo::RepoArgs, args: &CheckoutArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let pr = api
        .repo_get_pull_request()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let branch = pr
        .head
        .as_ref()
        .and_then(|h| h.ref_.as_deref())
        .ok_or_else(|| eyre::eyre!("PR has no head branch"))?;

    let status = std::process::Command::new("git")
        .args(["fetch", "origin", &format!("pull/{}/head:{branch}", args.number)])
        .status()
        .map_err(|e| eyre::eyre!("git fetch failed: {e}"))?;

    if !status.success() {
        let status = std::process::Command::new("git")
            .args(["fetch", "origin", branch])
            .status()
            .map_err(|e| eyre::eyre!("git fetch failed: {e}"))?;
        if !status.success() {
            eyre::bail!("Failed to fetch PR branch");
        }
    }

    let status = std::process::Command::new("git")
        .args(["checkout", branch])
        .status()
        .map_err(|e| eyre::eyre!("git checkout failed: {e}"))?;

    if !status.success() {
        eyre::bail!("Failed to checkout branch {branch}");
    }

    eprintln!("Checked out PR #{} on branch {branch}", args.number);
    Ok(())
}

async fn merge_pr(repo_args: &repo::RepoArgs, args: &MergeArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let do_method = match args.method.as_str() {
        "merge" | "rebase" | "squash" => args.method.as_str(),
        other => eyre::bail!("Invalid merge method: {other}. Use merge, rebase, or squash"),
    };

    api.repo_merge_pull_request()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .body_map(|b| {
            b.do_(do_method.to_string())
                .delete_branch_after_merge(args.delete_branch)
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("PR #{} merged ({do_method})", args.number);
    Ok(())
}

async fn set_pr_state(repo_opt: Option<&str>, number: i64, state: &str) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_opt, &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_edit_pull_request()
        .owner(owner)
        .repo(repo)
        .index(number)
        .body_map(|b| b.state(state.to_string()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("PR #{number} {state}");
    Ok(())
}

async fn comment_pr(repo_args: &repo::RepoArgs, args: &CommentArgs) -> Result<()> {
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

    eprintln!("Comment added to PR #{}", args.number);
    Ok(())
}

async fn review_pr(repo_args: &repo::RepoArgs, args: &ReviewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let event = match args.action.as_str() {
        "approve" => gitea_api::types::ReviewStateType::Approved,
        "request-changes" | "request_changes" => gitea_api::types::ReviewStateType::RequestChanges,
        "comment" => gitea_api::types::ReviewStateType::Comment,
        other => eyre::bail!("Invalid review action: {other}. Use approve, request-changes, or comment"),
    };

    let action_str = args.action.clone();
    api.repo_create_pull_review()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .body_map(move |b| b.body(args.body.clone()).event(event.clone()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Review submitted on PR #{}: {action_str}", args.number);
    Ok(())
}

async fn checks_pr(repo_args: &repo::RepoArgs, args: &ChecksArgs) -> Result<()> {
    let config = Config::load()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    // Use reqwest to get commit statuses for the PR's head SHA
    let api = config.client()?;
    let pr = api
        .repo_get_pull_request()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .index(args.number)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let sha = pr
        .head
        .as_ref()
        .and_then(|h| h.sha.as_deref())
        .ok_or_else(|| eyre::eyre!("PR has no head SHA"))?;

    // Get combined status for the SHA
    let combined = api
        .repo_get_combined_status_by_ref()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .ref_(sha)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&combined)?);
        return Ok(());
    }

    let state = combined.state.as_ref().map(|s| format!("{s:?}")).unwrap_or_else(|| "unknown".to_string()).to_lowercase();
    println!("Overall: {state}");

    if combined.statuses.is_empty() {
        println!("No checks found");
    }
    for s in &combined.statuses {
        let context = s.context.as_deref().unwrap_or("");
        let s_status = s.status.as_ref().map(|st| format!("{st:?}")).unwrap_or_default().to_lowercase();
        let desc = s.description.as_deref().unwrap_or("");
        println!("  {s_status:<10} {context} — {desc}");
    }

    Ok(())
}

async fn diff_pr(repo_args: &repo::RepoArgs, args: &DiffArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    // The diff endpoint returns plain text, not JSON.
    // Use raw_get since the typed client expects JSON responses.
    let path = format!(
        "repos/{}/{}/pulls/{}.diff",
        repo_info.owner, repo_info.name, args.number,
    );
    let text = api.raw_get(&path).await.map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;
    print!("{text}");
    Ok(())
}

async fn edit_pr(repo_args: &repo::RepoArgs, args: &EditArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // Resolve label names to IDs if provided
    let label_ids = if args.label.is_empty() {
        vec![]
    } else {
        let labels = api
            .issue_list_labels()
            .owner(owner)
            .repo(repo)
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner();

        let mut ids = Vec::new();
        for name in &args.label {
            match labels.iter().find(|l| l.name.as_deref() == Some(name.as_str())) {
                Some(l) => ids.push(l.id.unwrap_or(0)),
                None => eyre::bail!("Label not found: {name}"),
            }
        }
        ids
    };

    // Resolve milestone name to ID if provided
    let milestone_id = if let Some(ref ms_name) = args.milestone {
        let milestones = api
            .issue_get_milestones_list()
            .owner(owner)
            .repo(repo)
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner();
        match milestones.iter().find(|m| m.title.as_deref() == Some(ms_name.as_str())) {
            Some(m) => Some(m.id.unwrap_or(0)),
            None => eyre::bail!("Milestone not found: {ms_name}"),
        }
    } else {
        None
    };

    api.repo_edit_pull_request()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .body_map(|mut b| {
            if let Some(ref title) = args.title {
                b = b.title(title.clone());
            }
            if let Some(ref body) = args.body {
                b = b.body(body.clone());
            }
            if !label_ids.is_empty() {
                b = b.labels(label_ids.clone());
            }
            if !args.assignee.is_empty() {
                b = b.assignees(args.assignee.clone());
            }
            if let Some(ms) = milestone_id {
                b = b.milestone(ms);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Updated PR #{}", args.number);
    Ok(())
}

async fn update_branch(repo_args: &repo::RepoArgs, args: &UpdateBranchArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_update_pull_request()
        .owner(owner)
        .repo(repo)
        .index(args.number)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Updated branch for PR #{}", args.number);
    Ok(())
}
