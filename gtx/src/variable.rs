use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::atty_check;
use crate::repo;

#[derive(Args)]
pub struct VariableCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: VariableAction,
}

#[derive(Subcommand)]
enum VariableAction {
    /// List variables
    List(ListArgs),
    /// Get a variable value
    Get(GetArgs),
    /// Set a variable (creates or updates)
    Set(SetArgs),
    /// Delete a variable
    Delete(DeleteArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct GetArgs {
    /// Variable name
    name: String,
}

#[derive(Args)]
struct SetArgs {
    /// Variable name
    name: String,

    /// Variable value
    value: String,
}

#[derive(Args)]
struct DeleteArgs {
    /// Variable name
    name: String,
}

impl VariableCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            VariableAction::List(args) => list_variables(&self.repo, args).await,
            VariableAction::Get(args) => get_variable(&self.repo, args).await,
            VariableAction::Set(args) => set_variable(&self.repo, args).await,
            VariableAction::Delete(args) => delete_variable(&self.repo, args).await,
        }
    }
}

const VARIABLE_FIELDS: &[&str] = &["name", "data", "owner_id"];

async fn list_variables(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let vars = api
        .get_repo_variables_list()
        .owner(owner)
        .repo(repo)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &vars, VARIABLE_FIELDS);
    }

    if vars.is_empty() {
        eprintln!("No variables found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<30} {}", "NAME", "VALUE");
    }

    for v in &vars {
        let name = v.name.as_deref().unwrap_or("");
        let data = v.data.as_deref().unwrap_or("");
        println!("{:<30} {}", name, data);
    }

    Ok(())
}

async fn get_variable(repo_args: &repo::RepoArgs, args: &GetArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let var = api
        .get_repo_variable()
        .owner(owner)
        .repo(repo)
        .variablename(&args.name)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    println!("{}", var.data.as_deref().unwrap_or(""));
    Ok(())
}

async fn set_variable(repo_args: &repo::RepoArgs, args: &SetArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // Try update first, if 404 then create
    // Note: update returns 204 No Content which progenitor treats as an error,
    // so we check the status code to distinguish real errors from success.
    let update_result = api
        .update_repo_variable()
        .owner(owner)
        .repo(repo)
        .variablename(&args.name)
        .body_map(|b| b.value(args.value.clone()))
        .send()
        .await;

    match update_result {
        Ok(_) => {
            eprintln!("Variable '{}' updated", args.name);
        }
        Err(e) => {
            let err = gitea_api::GiteaError::from(e);
            let msg = err.to_string();
            if msg.contains("204") {
                // 204 No Content = success for updates
                eprintln!("Variable '{}' updated", args.name);
            } else if msg.contains("404") {
                // Create new variable
                api.create_repo_variable()
                    .owner(owner)
                    .repo(repo)
                    .variablename(&args.name)
                    .body_map(|b| b.value(args.value.clone()))
                    .send()
                    .await
                    .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;
                eprintln!("Variable '{}' created", args.name);
            } else {
                return Err(eyre::eyre!("{err}"));
            }
        }
    }

    Ok(())
}

async fn delete_variable(repo_args: &repo::RepoArgs, args: &DeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let result = api
        .delete_repo_variable()
        .owner(owner)
        .repo(repo)
        .variablename(&args.name)
        .send()
        .await;

    match result {
        Ok(_) => {}
        Err(e) => {
            let err = gitea_api::GiteaError::from(e);
            if !err.to_string().contains("204") {
                return Err(eyre::eyre!("{err}"));
            }
        }
    }

    eprintln!("Variable '{}' deleted", args.name);
    Ok(())
}
