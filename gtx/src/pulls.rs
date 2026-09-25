use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::json::{Field, field, gh};
use crate::paginate;
use crate::repo;
use gitea_api::types::{PrBranchInfo, PullRequest, PullReview, StateType};

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
    #[arg(short = 'L', long, default_value = "30")]
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

    #[command(flatten)]
    json: crate::json::JsonArgs,
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

    #[command(flatten)]
    json: crate::json::JsonArgs,
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

    /// Set the new title
    #[arg(short, long)]
    title: Option<String>,

    /// Set the new body
    #[arg(short, long, conflicts_with = "body_file")]
    body: Option<String>,

    /// Read body text from file
    #[arg(short = 'F', long)]
    body_file: Option<String>,

    /// Change the base branch for this pull request
    #[arg(short = 'B', long)]
    base: Option<String>,

    #[command(flatten)]
    meta: crate::issue_meta::EditMeta,
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
            PrAction::Close(args) => {
                set_pr_state(self.repo.repo.as_deref(), args.number, "closed").await
            }
            PrAction::Reopen(args) => {
                set_pr_state(self.repo.repo.as_deref(), args.number, "open").await
            }
            PrAction::Comment(args) => comment_pr(&self.repo, args).await,
            PrAction::Diff(args) => diff_pr(&self.repo, args).await,
            PrAction::Review(args) => review_pr(&self.repo, args).await,
            PrAction::Checks(args) => checks_pr(&self.repo, args).await,
            PrAction::Edit(args) => edit_pr(&self.repo, args).await,
            PrAction::UpdateBranch(args) => update_branch(&self.repo, args).await,
        }
    }
}

fn pr_closed(p: &PullRequest) -> bool {
    matches!(p.state, Some(StateType::Closed))
}

fn pr_head(p: &PullRequest) -> Option<&PrBranchInfo> {
    p.head.as_ref()
}

fn pr_base(p: &PullRequest) -> Option<&PrBranchInfo> {
    p.base.as_ref()
}

/// A branch's name. Gitea's `ref` becomes `refs/pull/N/head` once the head
/// branch is deleted, while `label` stays the branch name gh reports.
fn branch_name(b: Option<&PrBranchInfo>) -> serde_json::Value {
    gh::v(b.and_then(|b| b.label.as_deref().or(b.ref_.as_deref())))
}

/// gh's `pr list/view --json` fields that Gitea's pull request data can
/// answer, plus any `$extra` fields. Gitea has no GraphQL node IDs, so `id`
/// is the numeric ID. A macro so the same getters serve `PullRequest` and
/// [`PrView`] (which derefs to `PullRequest`).
macro_rules! pr_fields {
    ($($extra:expr),* $(,)?) => { &[
    field("additions", |p| gh::v(p.additions)),
    field("assignees", |p| gh::users(&p.assignees)),
    field("author", |p| gh::user(p.user.as_ref())),
    field("baseRefName", |p| branch_name(pr_base(p))),
    field("baseRefOid", |p| gh::v(pr_base(p).and_then(|b| b.sha.as_deref()))),
    field("body", |p| gh::v(p.body.as_deref().unwrap_or(""))),
    field("changedFiles", |p| gh::v(p.changed_files)),
    field("closed", |p| gh::v(pr_closed(p))),
    field("closedAt", |p| gh::time(p.closed_at)),
    field("createdAt", |p| gh::time(p.created_at)),
    field("deletions", |p| gh::v(p.deletions)),
    field("headRefName", |p| branch_name(pr_head(p))),
    field("headRefOid", |p| gh::v(pr_head(p).and_then(|h| h.sha.as_deref()))),
    field("headRepository", |p| {
        pr_head(p).and_then(|h| h.repo.as_ref()).map_or(serde_json::Value::Null, |r| {
            serde_json::json!({"id": r.id, "name": r.name})
        })
    }),
    field("headRepositoryOwner", |p| {
        gh::user(pr_head(p).and_then(|h| h.repo.as_ref()).and_then(|r| r.owner.as_ref()))
    }),
    field("id", |p| gh::v(p.id)),
    field("isCrossRepository", |p| {
        gh::v(pr_head(p).and_then(|h| h.repo_id) != pr_base(p).and_then(|b| b.repo_id))
    }),
    field("isDraft", |p| gh::v(p.draft.unwrap_or(false))),
    field("labels", |p| gh::labels(&p.labels)),
    field("maintainerCanModify", |p| gh::v(p.allow_maintainer_edit.unwrap_or(false))),
    field("mergeCommit", |p| {
        p.merge_commit_sha
            .as_ref()
            .map_or(serde_json::Value::Null, |oid| serde_json::json!({"oid": oid}))
    }),
    field("mergeable", |p| {
        gh::v(match p.mergeable {
            Some(true) => "MERGEABLE",
            Some(false) => "CONFLICTING",
            None => "UNKNOWN",
        })
    }),
    field("mergedAt", |p| gh::time(p.merged_at)),
    field("mergedBy", |p| gh::user(p.merged_by.as_ref())),
    field("milestone", |p| gh::milestone(p.milestone.as_ref())),
    field("number", |p| gh::v(p.number)),
    field("reviewRequests", |p| gh::users(&p.requested_reviewers)),
    field("state", |p| {
        gh::v(if p.merged.unwrap_or(false) {
            "MERGED"
        } else if pr_closed(p) {
            "CLOSED"
        } else {
            "OPEN"
        })
    }),
    field("title", |p| gh::v(&p.title)),
    field("updatedAt", |p| gh::time(p.updated_at)),
    field("url", |p| gh::v(&p.html_url)),
    $($extra),*
    ] };
}

const PR_FIELDS: &[Field<PullRequest>] = pr_fields!();

/// A pull request plus what `pr view --json` may fetch besides it.
#[derive(Default)]
struct PrView {
    pr: PullRequest,
    comments: Vec<gitea_api::types::Comment>,
    commits: Vec<gitea_api::types::Commit>,
    files: Vec<gitea_api::types::ChangedFile>,
    reviews: Vec<PullReview>,
}

impl std::ops::Deref for PrView {
    type Target = PullRequest;
    fn deref(&self) -> &PullRequest {
        &self.pr
    }
}

const PR_VIEW_FIELDS: &[Field<PrView>] = pr_fields![
    field("comments", |v| gh::comments(&v.comments)),
    field("commits", |v| serde_json::Value::Array(
        v.commits.iter().map(gh_commit).collect()
    )),
    field("files", |v| {
        serde_json::Value::Array(
            v.files
                .iter()
                .map(|f| serde_json::json!({"path": f.filename, "additions": f.additions, "deletions": f.deletions}))
                .collect(),
        )
    }),
    field("latestReviews", |v| {
        serde_json::Value::Array(
            latest_reviews(&v.reviews)
                .into_iter()
                .map(gh_review)
                .collect(),
        )
    }),
    field("reviews", |v| {
        serde_json::Value::Array(submitted_reviews(&v.reviews).map(gh_review).collect())
    }),
];

/// gh's commit object for `pr view --json commits`.
fn gh_commit(c: &gitea_api::types::Commit) -> serde_json::Value {
    let rc = c.commit.as_ref();
    let message = rc.and_then(|m| m.message.as_deref()).unwrap_or("");
    let (headline, body) = message.split_once('\n').unwrap_or((message, ""));
    let date = |u: Option<&gitea_api::types::CommitUser>| {
        u.and_then(|u| u.date.as_deref())
            .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
            .map_or(serde_json::Value::Null, |d| gh::time(Some(d.to_utc())))
    };
    let author = rc.and_then(|m| m.author.as_ref());
    let login = c.author.as_ref();
    serde_json::json!({
        "oid": c.sha,
        "messageHeadline": headline.trim_end(),
        "messageBody": body.trim(),
        "authoredDate": date(author),
        "committedDate": date(rc.and_then(|m| m.committer.as_ref())),
        "authors": [{
            "email": author.and_then(|a| a.email.as_deref()).unwrap_or(""),
            "id": login.and_then(|u| u.id),
            "login": login.and_then(|u| u.login.as_deref()).unwrap_or(""),
            "name": author.and_then(|a| a.name.as_deref()).unwrap_or(""),
        }],
    })
}

/// gh's review state names for Gitea's, or `None` for Gitea's
/// review-request placeholders, which gh doesn't count as reviews.
fn gh_review_state(r: &PullReview) -> Option<&'static str> {
    use gitea_api::types::ReviewStateType as S;
    if r.dismissed.unwrap_or(false) {
        return Some("DISMISSED");
    }
    match r.state.as_ref()? {
        S::Approved => Some("APPROVED"),
        S::Pending => Some("PENDING"),
        S::Comment => Some("COMMENTED"),
        S::RequestChanges => Some("CHANGES_REQUESTED"),
        S::RequestReview => None,
    }
}

fn submitted_reviews(rs: &[PullReview]) -> impl Iterator<Item = &PullReview> {
    rs.iter().filter(|r| gh_review_state(r).is_some())
}

/// Each reviewer's most recent review, in review order.
fn latest_reviews(rs: &[PullReview]) -> Vec<&PullReview> {
    let mut latest: Vec<&PullReview> = Vec::new();
    for r in submitted_reviews(rs) {
        let who = r.user.as_ref().and_then(|u| u.id);
        latest.retain(|l| l.user.as_ref().and_then(|u| u.id) != who);
        latest.push(r);
    }
    latest
}

fn gh_review(r: &PullReview) -> serde_json::Value {
    serde_json::json!({
        "id": r.id,
        "author": gh::author(r.user.as_ref()),
        "body": r.body.as_deref().unwrap_or(""),
        "state": gh_review_state(r),
        "submittedAt": gh::time(r.submitted_at),
        "commit": {"oid": r.commit_id},
    })
}

/// gh's `pr checks --json` fields, from Gitea's commit statuses.
const CHECK_FIELDS: &[Field<gitea_api::types::CommitStatus>] = &[
    field("bucket", |s| {
        use gitea_api::types::CommitStatusState as S;
        gh::v(match s.status {
            Some(S::Success) => "pass",
            Some(S::Failure | S::Error) => "fail",
            Some(S::Skipped) => "skipping",
            _ => "pending",
        })
    }),
    field("completedAt", |s| gh::time(s.updated_at)),
    field("description", |s| {
        gh::v(s.description.as_deref().unwrap_or(""))
    }),
    field("link", |s| gh::v(s.target_url.as_deref().unwrap_or(""))),
    field("name", |s| gh::v(s.context.as_deref().unwrap_or(""))),
    field("startedAt", |s| gh::time(s.created_at)),
    field("state", |s| {
        gh::v(
            s.status
                .as_ref()
                .map(|st| gh::v(st).as_str().unwrap_or("").to_uppercase()),
        )
    }),
];

async fn list_prs(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let json = args.json.select(PR_FIELDS)?;
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
                _ => req = req.state(gitea_api::types::RepoListPullRequestsState::All),
            }
            Ok(req
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner())
        }
    })
    .await?;

    if let Some(json) = json {
        return json.write_list(&prs);
    }

    if prs.is_empty() {
        eprintln!("No pull requests found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<6} {:<50} {:<15} {}", "#", "TITLE", "AUTHOR", "UPDATED");
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
    let json = args.json.select(PR_VIEW_FIELDS)?;
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

    if let Some(json) = json {
        let n = args.number;
        let mut view = PrView {
            pr,
            ..Default::default()
        };
        let api_err = |e| eyre::eyre!("{}", gitea_api::GiteaError::from(e));
        if json.wants("comments") {
            view.comments = api
                .issue_get_comments()
                .owner(owner)
                .repo(repo)
                .index(n)
                .send()
                .await
                .map_err(api_err)?
                .into_inner();
        }
        if json.wants("commits") {
            view.commits = paginate::paginate(10_000, 50, |page, limit| {
                let api = &api;
                async move {
                    Ok(api
                        .repo_get_pull_request_commits()
                        .owner(owner)
                        .repo(repo)
                        .index(n)
                        .files(false)
                        .verification(false)
                        .page(page)
                        .limit(limit)
                        .send()
                        .await
                        .map_err(api_err)?
                        .into_inner())
                }
            })
            .await?;
        }
        if json.wants("files") {
            view.files = paginate::paginate(10_000, 50, |page, limit| {
                let api = &api;
                async move {
                    Ok(api
                        .repo_get_pull_request_files()
                        .owner(owner)
                        .repo(repo)
                        .index(n)
                        .page(page)
                        .limit(limit)
                        .send()
                        .await
                        .map_err(api_err)?
                        .into_inner())
                }
            })
            .await?;
        }
        if json.wants("reviews") || json.wants("latestReviews") {
            view.reviews = paginate::paginate(10_000, 50, |page, limit| {
                let api = &api;
                async move {
                    Ok(api
                        .repo_list_pull_reviews()
                        .owner(owner)
                        .repo(repo)
                        .index(n)
                        .page(page)
                        .limit(limit)
                        .send()
                        .await
                        .map_err(api_err)?
                        .into_inner())
                }
            })
            .await?;
        }
        return json.write_one(&view);
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
                let when = c.created_at.map(|dt| relative_time(dt)).unwrap_or_default();
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
            let new_body = crate::body::upload_and_rewrite(
                &api, &config, owner, repo, number, &body, base_dir,
            )
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
                Ok(inquire::validator::Validation::Invalid(
                    "Title is required".into(),
                ))
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
        .args([
            "fetch",
            "origin",
            &format!("pull/{}/head:{branch}", args.number),
        ])
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
        other => {
            eyre::bail!("Invalid review action: {other}. Use approve, request-changes, or comment")
        }
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
    let json = args.json.select(CHECK_FIELDS)?;
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

    if let Some(json) = json {
        return json.write_list(&combined.statuses);
    }

    let state = combined
        .state
        .as_ref()
        .map(|s| format!("{s:?}"))
        .unwrap_or_else(|| "unknown".to_string())
        .to_lowercase();
    println!("Overall: {state}");

    if combined.statuses.is_empty() {
        println!("No checks found");
    }
    for s in &combined.statuses {
        let context = s.context.as_deref().unwrap_or("");
        let s_status = s
            .status
            .as_ref()
            .map(|st| format!("{st:?}"))
            .unwrap_or_default()
            .to_lowercase();
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
    let text = api
        .raw_get(&path)
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;
    print!("{text}");
    Ok(())
}

async fn edit_pr(repo_args: &repo::RepoArgs, args: &EditArgs) -> Result<()> {
    let body = match args.body_file {
        Some(ref path) => Some(crate::body::read_body_file(path)?),
        None => args.body.clone(),
    };
    if args.title.is_none() && body.is_none() && args.base.is_none() && args.meta.is_empty() {
        eyre::bail!("field to edit flag required when not running interactively");
    }

    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    if let Some(ref base) = args.base {
        api.repo_edit_pull_request()
            .owner(owner)
            .repo(repo)
            .index(args.number)
            .body_map(|b| b.base(base.clone()))
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;
    }

    let issue = crate::issue_meta::edit(
        &api,
        owner,
        repo,
        args.number,
        args.title.as_deref(),
        body.as_deref(),
        &args.meta,
    )
    .await?;
    println!("{}", issue.html_url.as_deref().unwrap_or_default());
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
