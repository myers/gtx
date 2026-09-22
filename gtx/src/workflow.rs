use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::atty_check;
use crate::repo;

#[derive(Args)]
pub struct WorkflowCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: WorkflowAction,
}

#[derive(Subcommand)]
enum WorkflowAction {
    /// List workflows
    List(ListArgs),
    /// Trigger a workflow dispatch
    Run(RunArgs),
    /// Enable a workflow
    Enable(ToggleArgs),
    /// Disable a workflow
    Disable(ToggleArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct RunArgs {
    /// Workflow ID (filename, e.g. "build.yml")
    workflow: String,

    /// Branch or tag to run on
    #[arg(short = 'r', long, default_value = "main")]
    ref_: String,

    /// Input parameters (key=value, can be repeated)
    #[arg(short, long, value_parser = parse_input)]
    input: Vec<(String, String)>,
}

#[derive(Args)]
struct ToggleArgs {
    /// Workflow ID (filename, e.g. "build.yml")
    workflow: String,
}

fn parse_input(s: &str) -> Result<(String, String), String> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| format!("Invalid input format: {s}. Use key=value"))?;
    Ok((key.to_string(), value.to_string()))
}

impl WorkflowCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            WorkflowAction::List(args) => list_workflows(&self.repo, args).await,
            WorkflowAction::Run(args) => run_workflow(&self.repo, args).await,
            WorkflowAction::Enable(args) => enable_workflow(&self.repo, args).await,
            WorkflowAction::Disable(args) => disable_workflow(&self.repo, args).await,
        }
    }
}

const WORKFLOW_FIELDS: &[&str] = &[
    "id", "name", "path", "state", "html_url", "badge_url",
    "created_at", "updated_at",
];

async fn list_workflows(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let result = api
        .actions_list_repository_workflows()
        .owner(owner)
        .repo(repo)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let workflows = &result.workflows;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, workflows, WORKFLOW_FIELDS);
    }

    if workflows.is_empty() {
        eprintln!("No workflows found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<25} {:<35} {:<10} {}", "ID", "NAME", "STATE", "PATH");
    }

    for wf in workflows {
        let id = wf.id.as_deref().unwrap_or("");
        let name = wf.name.as_deref().unwrap_or("");
        let truncated_name = if name.len() > 33 {
            format!("{}...", &name[..30])
        } else {
            name.to_string()
        };
        let state = wf.state.as_deref().unwrap_or("");
        let path = wf.path.as_deref().unwrap_or("");

        println!("{:<25} {:<35} {:<10} {}", id, truncated_name, state, path);
    }

    Ok(())
}

async fn run_workflow(repo_args: &repo::RepoArgs, args: &RunArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let inputs: std::collections::HashMap<String, String> =
        args.input.iter().cloned().collect();

    let result = api
        .actions_dispatch_workflow()
        .owner(owner)
        .repo(repo)
        .workflow_id(&args.workflow)
        .body_map(|mut b| {
            b = b.ref_(args.ref_.clone());
            if !inputs.is_empty() {
                b = b.inputs(inputs.clone());
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if let Some(run_id) = result.workflow_run_id {
        let url = result.html_url.as_deref().unwrap_or("");
        eprintln!("Triggered workflow '{}' → run #{run_id}", args.workflow);
        if !url.is_empty() {
            eprintln!("{url}");
        }
    } else {
        eprintln!("Triggered workflow '{}'", args.workflow);
    }

    Ok(())
}

async fn enable_workflow(repo_args: &repo::RepoArgs, args: &ToggleArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let result = api
        .actions_enable_workflow()
        .owner(owner)
        .repo(repo)
        .workflow_id(&args.workflow)
        .send()
        .await;

    // 204 No Content is success
    match result {
        Ok(_) => {}
        Err(e) => {
            let err = gitea_api::GiteaError::from(e);
            if !err.to_string().contains("204") {
                return Err(eyre::eyre!("{err}"));
            }
        }
    }

    eprintln!("Enabled workflow '{}'", args.workflow);
    Ok(())
}

async fn disable_workflow(repo_args: &repo::RepoArgs, args: &ToggleArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let result = api
        .actions_disable_workflow()
        .owner(owner)
        .repo(repo)
        .workflow_id(&args.workflow)
        .send()
        .await;

    // 204 No Content is success
    match result {
        Ok(_) => {}
        Err(e) => {
            let err = gitea_api::GiteaError::from(e);
            if !err.to_string().contains("204") {
                return Err(eyre::eyre!("{err}"));
            }
        }
    }

    eprintln!("Disabled workflow '{}'", args.workflow);
    Ok(())
}
