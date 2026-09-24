use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::atty_check;
use crate::json::{Field, field, gh};
use crate::repo;
use gitea_api::types::{Project, ProjectColumn};

#[derive(Args)]
pub struct ProjectCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: ProjectAction,
}

#[derive(Subcommand)]
enum ProjectAction {
    /// List projects
    List(ListArgs),
    /// View a project
    View(ViewArgs),
    /// Create a project
    Create(CreateArgs),
    /// Close a project
    Close(StateArgs),
    /// Reopen a project
    Reopen(StateArgs),
    /// Manage project columns
    Column(ColumnCommand),
}

#[derive(Args)]
struct ColumnCommand {
    #[command(subcommand)]
    action: ColumnAction,
}

#[derive(Subcommand)]
enum ColumnAction {
    /// List columns in a project
    List(ColumnListArgs),
    /// Create a column in a project
    Create(ColumnCreateArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct ViewArgs {
    id: i64,
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct CreateArgs {
    #[arg(short, long)]
    title: String,
    #[arg(short, long)]
    description: Option<String>,
}

#[derive(Args)]
struct StateArgs {
    id: i64,
}

#[derive(Args)]
struct ColumnListArgs {
    project_id: i64,
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct ColumnCreateArgs {
    project_id: i64,
    #[arg(short, long)]
    title: String,
    #[arg(short, long)]
    color: Option<String>,
}

impl ProjectCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            ProjectAction::List(args) => list_projects(&self.repo, args).await,
            ProjectAction::View(args) => view_project(&self.repo, args).await,
            ProjectAction::Create(args) => create_project(&self.repo, args).await,
            ProjectAction::Close(args) => set_project_state(&self.repo, args, true).await,
            ProjectAction::Reopen(args) => set_project_state(&self.repo, args, false).await,
            ProjectAction::Column(cmd) => match &cmd.action {
                ColumnAction::List(args) => list_columns(&self.repo, args).await,
                ColumnAction::Create(args) => create_column(&self.repo, args).await,
            },
        }
    }
}

/// `project list/view --json` fields (Gitea's repo projects, which gh's
/// `project` doesn't model; camelCase like the rest).
const PROJECT_FIELDS: &[Field<Project>] = &[
    field("closedAt", |p| gh::time(p.closed_at)),
    field("closedIssues", |p| gh::v(p.closed_issues.unwrap_or(0))),
    field("createdAt", |p| gh::time(p.created_at)),
    field("description", |p| {
        gh::v(p.description.as_deref().unwrap_or(""))
    }),
    field("id", |p| gh::v(p.id)),
    field("openIssues", |p| gh::v(p.open_issues.unwrap_or(0))),
    field("state", |p| {
        gh::v(
            if matches!(p.state, Some(gitea_api::types::StateType::Closed)) {
                "CLOSED"
            } else {
                "OPEN"
            },
        )
    }),
    field("title", |p| gh::v(&p.title)),
    field("updatedAt", |p| gh::time(p.updated_at)),
];

async fn list_projects(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let json = args.json.select(PROJECT_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let projects = api
        .project_list_projects()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .page(1)
        .limit(50)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if let Some(json) = json {
        return json.write_list(&projects);
    }

    if projects.is_empty() {
        eprintln!("No projects found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<6} {:<40} {}", "ID", "TITLE", "STATE");
    }

    for p in &projects {
        let id = p.id.unwrap_or(0);
        let title = p.title.as_deref().unwrap_or("");
        let state = p
            .state
            .as_ref()
            .map(|s| format!("{s:?}"))
            .unwrap_or_default()
            .to_lowercase();
        println!("{:<6} {:<40} {}", id, title, state);
    }

    Ok(())
}

async fn view_project(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let json = args.json.select(PROJECT_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let project = api
        .project_get_project()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .project_id(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if let Some(json) = json {
        return json.write_one(&project);
    }

    let title = project.title.as_deref().unwrap_or("(no title)");
    let id = project.id.unwrap_or(0);
    let state = project
        .state
        .as_ref()
        .map(|s| format!("{s:?}"))
        .unwrap_or_default()
        .to_lowercase();

    println!("{title} (#{id})");
    println!("{state}");

    if let Some(ref desc) = project.description {
        if !desc.is_empty() {
            println!();
            println!("{desc}");
        }
    }

    Ok(())
}

async fn create_project(repo_args: &repo::RepoArgs, args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let desc = args.description.clone();
    let project = api
        .project_create_project()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .body_map(|mut b| {
            b = b.title(args.title.clone());
            if let Some(d) = desc {
                b = b.description(d);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let id = project.id.unwrap_or(0);
    let title = project.title.as_deref().unwrap_or("");
    eprintln!("Created project #{id}: {title}");
    Ok(())
}

async fn set_project_state(
    repo_args: &repo::RepoArgs,
    args: &StateArgs,
    close: bool,
) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let state = if close {
        gitea_api::types::StateType::Closed
    } else {
        gitea_api::types::StateType::Open
    };

    api.project_edit_project()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .project_id(args.id)
        .body_map(|b| b.state(state.clone()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    let label = if close { "closed" } else { "reopened" };
    eprintln!("Project #{} {label}", args.id);
    Ok(())
}

/// `project column list --json` fields.
const COLUMN_FIELDS: &[Field<ProjectColumn>] = &[
    field("color", |c| gh::v(c.color.as_deref().unwrap_or(""))),
    field("createdAt", |c| gh::time(c.created_at)),
    field("id", |c| gh::v(c.id)),
    field("isDefault", |c| gh::v(c.default.unwrap_or(false))),
    field("sorting", |c| gh::v(c.sorting)),
    field("title", |c| gh::v(&c.title)),
    field("updatedAt", |c| gh::time(c.updated_at)),
];

async fn list_columns(repo_args: &repo::RepoArgs, args: &ColumnListArgs) -> Result<()> {
    let json = args.json.select(COLUMN_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let columns = api
        .project_list_columns()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .project_id(args.project_id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if let Some(json) = json {
        return json.write_list(&columns);
    }

    if columns.is_empty() {
        eprintln!("No columns found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<6} {:<30} {}", "ID", "TITLE", "COLOR");
    }

    for col in &columns {
        let id = col.id.unwrap_or(0);
        let title = col.title.as_deref().unwrap_or("");
        let color = col.color.as_deref().unwrap_or("");
        println!("{:<6} {:<30} {}", id, title, color);
    }

    Ok(())
}

async fn create_column(repo_args: &repo::RepoArgs, args: &ColumnCreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let color = args.color.clone();
    let column = api
        .project_create_column()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .project_id(args.project_id)
        .body_map(|mut b| {
            b = b.title(args.title.clone());
            if let Some(c) = color {
                b = b.color(c);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let id = column.id.unwrap_or(0);
    let title = column.title.as_deref().unwrap_or("");
    eprintln!("Created column #{id}: {title}");
    Ok(())
}
