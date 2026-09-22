use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::paginate;
use crate::repo;

#[derive(Args)]
pub struct MilestoneCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: MilestoneAction,
}

#[derive(Subcommand)]
enum MilestoneAction {
    /// List milestones
    List(ListArgs),
    /// Create a milestone
    Create(CreateArgs),
    /// View a milestone
    View(ViewArgs),
    /// Close a milestone
    Close(CloseArgs),
    /// Reopen a milestone
    Reopen(ReopenArgs),
}

#[derive(Args)]
struct ListArgs {
    /// Filter by state (open, closed, all)
    #[arg(short, long, default_value = "open")]
    state: String,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct CreateArgs {
    /// Milestone title
    #[arg(short, long)]
    title: String,

    /// Milestone description
    #[arg(short, long)]
    description: Option<String>,
}

#[derive(Args)]
struct ViewArgs {
    /// Milestone ID
    id: i64,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct CloseArgs {
    /// Milestone ID
    id: i64,
}

#[derive(Args)]
struct ReopenArgs {
    /// Milestone ID
    id: i64,
}

impl MilestoneCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            MilestoneAction::List(args) => list_milestones(&self.repo, args).await,
            MilestoneAction::Create(args) => create_milestone(&self.repo, args).await,
            MilestoneAction::View(args) => view_milestone(&self.repo, args).await,
            MilestoneAction::Close(args) => {
                set_milestone_state(
                    self.repo.repo.as_deref(),
                    args.id,
                    gitea_api::types::StateType::Closed,
                )
                .await
            }
            MilestoneAction::Reopen(args) => {
                set_milestone_state(
                    self.repo.repo.as_deref(),
                    args.id,
                    gitea_api::types::StateType::Open,
                )
                .await
            }
        }
    }
}

const MILESTONE_FIELDS: &[&str] = &[
    "id", "title", "description", "state", "open_issues", "closed_issues",
    "due_on", "created_at", "updated_at", "closed_at",
];

async fn list_milestones(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
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
    let milestones = paginate::paginate(200, 50, |page, per_page| {
        let api = &api;
        let state_str = &state_str;
        async move {
            Ok(api
                .issue_get_milestones_list()
                .owner(owner)
                .repo(repo)
                .page(page)
                .limit(per_page)
                .state(state_str.clone())
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner())
        }
    })
    .await?;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &milestones, &MILESTONE_FIELDS);
    }

    if milestones.is_empty() {
        eprintln!("No milestones found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<6} {:<40} {:<8} {:<8} {}",
            "ID", "TITLE", "OPEN", "CLOSED", "UPDATED"
        );
    }

    for ms in &milestones {
        let id = ms.id.unwrap_or(0);
        let title = ms.title.as_deref().unwrap_or("");
        let truncated_title = if title.len() > 38 {
            format!("{}...", &title[..35])
        } else {
            title.to_string()
        };
        let open = ms.open_issues.unwrap_or(0);
        let closed = ms.closed_issues.unwrap_or(0);
        let updated = ms
            .updated_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();

        println!(
            "{:<6} {:<40} {:<8} {:<8} {}",
            id, truncated_title, open, closed, updated
        );
    }

    Ok(())
}

async fn create_milestone(repo_args: &repo::RepoArgs, args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let description = args.description.clone();
    let ms = api
        .issue_create_milestone()
        .owner(owner)
        .repo(repo)
        .body_map(|mut b| {
            b = b.title(args.title.clone());
            if let Some(desc) = description {
                b = b.description(desc);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let id = ms.id.unwrap_or(0);
    let title = ms.title.as_deref().unwrap_or("");
    eprintln!("Created milestone #{id}: {title}");
    Ok(())
}

async fn view_milestone(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let ms = api
        .issue_get_milestone()
        .owner(owner)
        .repo(repo)
        .id(args.id.to_string())
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&ms)?);
        return Ok(());
    }

    let id = ms.id.unwrap_or(0);
    let title = ms.title.as_deref().unwrap_or("(no title)");
    let state = ms
        .state
        .as_ref()
        .map(|s| format!("{s:?}"))
        .unwrap_or_default()
        .to_lowercase();
    let open = ms.open_issues.unwrap_or(0);
    let closed = ms.closed_issues.unwrap_or(0);

    println!("{title} #{id}");
    println!("{state} -- {open} open, {closed} closed");

    if let Some(ref due) = ms.due_on {
        println!("Due: {}", due.format("%Y-%m-%d"));
    }

    if let Some(ref desc) = ms.description {
        if !desc.is_empty() {
            println!();
            println!("{desc}");
        }
    }

    Ok(())
}

async fn set_milestone_state(
    repo_opt: Option<&str>,
    id: i64,
    state: gitea_api::types::StateType,
) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_opt, &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.issue_edit_milestone()
        .owner(owner)
        .repo(repo)
        .id(id.to_string())
        .body_map(|b| b.state(state.clone()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Milestone #{id} {state}");
    Ok(())
}
