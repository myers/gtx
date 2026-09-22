use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::atty_check;
use crate::repo;

#[derive(Args)]
pub struct SecretCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: SecretAction,
}

#[derive(Subcommand)]
enum SecretAction {
    /// List secrets
    List(ListArgs),
    /// Set a secret (creates or updates)
    Set(SetArgs),
    /// Delete a secret
    Delete(DeleteArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct SetArgs {
    /// Secret name
    name: String,

    /// Secret value (reads from stdin if omitted)
    #[arg(long)]
    value: Option<String>,
}

#[derive(Args)]
struct DeleteArgs {
    /// Secret name
    name: String,
}

impl SecretCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            SecretAction::List(args) => list_secrets(&self.repo, args).await,
            SecretAction::Set(args) => set_secret(&self.repo, args).await,
            SecretAction::Delete(args) => delete_secret(&self.repo, args).await,
        }
    }
}

const SECRET_FIELDS: &[&str] = &["name", "created_at"];

async fn list_secrets(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let secrets = api
        .repo_list_actions_secrets()
        .owner(owner)
        .repo(repo)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &secrets, SECRET_FIELDS);
    }

    if secrets.is_empty() {
        eprintln!("No secrets found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<30} {}", "NAME", "UPDATED");
    }

    for s in &secrets {
        let name = s.name.as_deref().unwrap_or("");
        let updated = s
            .created_at
            .map(|dt| crate::issues::relative_time(dt))
            .unwrap_or_default();
        println!("{:<30} {}", name, updated);
    }

    Ok(())
}

async fn set_secret(repo_args: &repo::RepoArgs, args: &SetArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let value = match &args.value {
        Some(v) => v.clone(),
        None => {
            if atty_check() {
                eprintln!("Enter secret value (press Enter when done):");
            }
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
            buf.trim().to_string()
        }
    };

    api.update_repo_secret()
        .owner(owner)
        .repo(repo)
        .secretname(&args.name)
        .body_map(|b| b.data(value.clone()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Secret '{}' set", args.name);
    Ok(())
}

async fn delete_secret(repo_args: &repo::RepoArgs, args: &DeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.delete_repo_secret()
        .owner(owner)
        .repo(repo)
        .secretname(&args.name)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Secret '{}' deleted", args.name);
    Ok(())
}
