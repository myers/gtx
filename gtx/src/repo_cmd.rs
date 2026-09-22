use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::paginate;
use crate::repo;

#[derive(Args)]
pub struct RepoCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: RepoAction,
}

#[derive(Subcommand)]
enum RepoAction {
    /// View repository info
    View(ViewArgs),
    /// List repositories for a user or organization
    List(ListArgs),
    /// Clone a repository
    Clone(CloneArgs),
    /// Create a new repository
    Create(CreateArgs),
    /// Fork a repository
    Fork(ForkArgs),
    /// Delete a repository
    Delete(DeleteRepoArgs),
    /// Edit repository settings
    Edit(EditRepoArgs),
    /// Archive a repository
    Archive(ArchiveArgs),
    /// Unarchive a repository
    Unarchive(ArchiveArgs),
    /// Rename a repository
    Rename(RenameArgs),
    /// Manage deploy keys
    DeployKey(DeployKeyCommand),
}

#[derive(Args)]
struct ViewArgs {
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ListArgs {
    /// User or organization name (defaults to authenticated user)
    owner: Option<String>,

    /// Maximum number of repos to show
    #[arg(short, long, default_value = "30")]
    limit: i64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct CloneArgs {
    /// Repository to clone (owner/repo)
    repo: String,
}

#[derive(Args)]
struct CreateArgs {
    /// Repository name
    name: String,

    /// Repository description
    #[arg(short, long)]
    description: Option<String>,

    /// Make repository private
    #[arg(long)]
    private: bool,
}

#[derive(Args)]
struct ForkArgs {
    /// Repository to fork (owner/repo)
    repo: String,
}

#[derive(Args)]
struct DeleteRepoArgs {
    /// Confirm deletion (required)
    #[arg(long)]
    confirm: bool,
}

#[derive(Args)]
struct EditRepoArgs {
    /// New description
    #[arg(short, long)]
    description: Option<String>,

    /// Set default branch
    #[arg(long)]
    default_branch: Option<String>,

    /// Set visibility (true = private, false = public)
    #[arg(long)]
    private: Option<bool>,

    /// Set website URL
    #[arg(short, long)]
    website: Option<String>,
}

#[derive(Args)]
struct ArchiveArgs {}

#[derive(Args)]
struct RenameArgs {
    /// New repository name
    name: String,
}

#[derive(Args)]
pub struct DeployKeyCommand {
    #[command(subcommand)]
    action: DeployKeyAction,
}

#[derive(Subcommand)]
enum DeployKeyAction {
    /// List deploy keys
    List(DeployKeyListArgs),
    /// Add a deploy key
    Add(DeployKeyAddArgs),
    /// Delete a deploy key
    Delete(DeployKeyDeleteArgs),
}

#[derive(Args)]
struct DeployKeyListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct DeployKeyAddArgs {
    /// Key title
    #[arg(short, long)]
    title: String,

    /// SSH public key (or path to .pub file)
    key: String,

    /// Read-only access (default: true)
    #[arg(long, default_value = "true")]
    read_only: bool,
}

#[derive(Args)]
struct DeployKeyDeleteArgs {
    /// Key ID
    id: i64,
}

impl RepoCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            RepoAction::View(args) => view_repo(&self.repo, args).await,
            RepoAction::List(args) => list_repos(args).await,
            RepoAction::Clone(args) => clone_repo(args).await,
            RepoAction::Create(args) => create_repo(args).await,
            RepoAction::Fork(args) => fork_repo(args).await,
            RepoAction::Delete(args) => delete_repo(&self.repo, args).await,
            RepoAction::Edit(args) => edit_repo(&self.repo, args).await,
            RepoAction::Archive(_) => set_archived(&self.repo, true).await,
            RepoAction::Unarchive(_) => set_archived(&self.repo, false).await,
            RepoAction::Rename(args) => rename_repo(&self.repo, args).await,
            RepoAction::DeployKey(cmd) => deploy_key(&self.repo, cmd).await,
        }
    }
}

async fn view_repo(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let repo_data = api
        .repo_get()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&repo_data)?);
        return Ok(());
    }

    let name = repo_data.full_name.as_deref().unwrap_or("");
    let desc = repo_data.description.as_deref().unwrap_or("No description");
    let default_branch = repo_data.default_branch.as_deref().unwrap_or("main");
    let stars = repo_data.stars_count.unwrap_or(0);
    let forks = repo_data.forks_count.unwrap_or(0);
    let open_issues = repo_data.open_issues_count.unwrap_or(0);
    let private = repo_data.private.unwrap_or(false);

    println!("{name}");
    println!("{desc}");
    println!();
    println!(
        "{} — {} stars — {} forks — {} open issues — default: {default_branch}",
        if private { "Private" } else { "Public" },
        stars,
        forks,
        open_issues,
    );

    if let Some(ref url) = repo_data.html_url {
        println!("{url}");
    }

    Ok(())
}

const REPO_FIELDS: &[&str] = &[
    "id", "name", "full_name", "description", "private", "fork", "archived",
    "stars_count", "forks_count", "open_issues_count", "default_branch",
    "created_at", "updated_at", "html_url", "clone_url", "ssh_url",
];

async fn list_repos(args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repos = if let Some(ref owner) = args.owner {
        let owner = owner.as_str();
        paginate::paginate(args.limit, 50, |page, per_page| {
            let api = &api;
            async move {
                Ok(api
                    .user_list_repos()
                    .username(owner)
                    .page(page)
                    .limit(per_page)
                    .send()
                    .await
                    .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                    .into_inner())
            }
        })
        .await?
    } else {
        paginate::paginate(args.limit, 50, |page, per_page| {
            let api = &api;
            async move {
                Ok(api
                    .user_current_list_repos()
                    .page(page)
                    .limit(per_page)
                    .send()
                    .await
                    .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                    .into_inner())
            }
        })
        .await?
    };

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &repos, &REPO_FIELDS);
    }

    if repos.is_empty() {
        eprintln!("No repositories found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<40} {:<10} {}", "REPO", "STARS", "UPDATED");
    }

    for r in &repos {
        let name = r.full_name.as_deref().unwrap_or("");
        let stars = r.stars_count.unwrap_or(0);
        let updated = r
            .updated_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();
        println!("{:<40} {:<10} {}", name, stars, updated);
    }

    Ok(())
}

async fn clone_repo(args: &CloneArgs) -> Result<()> {
    let config = Config::load()?;

    let clone_url = format!(
        "{}{}.git",
        config.url.as_str().trim_end_matches('/'),
        if args.repo.starts_with('/') {
            args.repo.clone()
        } else {
            format!("/{}", args.repo)
        },
    );

    let status = std::process::Command::new("git")
        .args(["clone", &clone_url])
        .status()
        .map_err(|e| eyre::eyre!("git clone failed: {e}"))?;

    if !status.success() {
        eyre::bail!("git clone failed");
    }

    Ok(())
}

async fn create_repo(args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let description = args.description.clone();
    let private = args.private;

    let repo_data = api
        .create_current_user_repo()
        .body_map(|mut b| {
            b = b.name(args.name.clone()).private(private).auto_init(true);
            if let Some(desc) = description {
                b = b.description(desc);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let full_name = repo_data.full_name.as_deref().unwrap_or("");
    let url = repo_data.html_url.as_deref().unwrap_or("");
    eprintln!("Created repository {full_name}: {url}");
    Ok(())
}

async fn fork_repo(args: &ForkArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::parse_repo(&args.repo)?;

    let forked = api
        .create_fork()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .body_map(|b| b)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let full_name = forked.full_name.as_deref().unwrap_or("");
    let url = forked.html_url.as_deref().unwrap_or("");
    eprintln!("Forked to {full_name}: {url}");
    Ok(())
}

async fn delete_repo(repo_args: &repo::RepoArgs, args: &DeleteRepoArgs) -> Result<()> {
    if !args.confirm {
        eyre::bail!("Use --confirm to delete the repository. This cannot be undone.");
    }

    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_delete()
        .owner(owner)
        .repo(repo)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Deleted repository {owner}/{repo}");
    Ok(())
}

async fn edit_repo(repo_args: &repo::RepoArgs, args: &EditRepoArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_edit()
        .owner(owner)
        .repo(repo)
        .body_map(|mut b| {
            if let Some(ref desc) = args.description {
                b = b.description(desc.clone());
            }
            if let Some(ref branch) = args.default_branch {
                b = b.default_branch(branch.clone());
            }
            if let Some(private) = args.private {
                b = b.private(private);
            }
            if let Some(ref website) = args.website {
                b = b.website(website.clone());
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Updated repository {owner}/{repo}");
    Ok(())
}

async fn set_archived(repo_args: &repo::RepoArgs, archived: bool) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_edit()
        .owner(owner)
        .repo(repo)
        .body_map(|b| b.archived(archived))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    let action = if archived { "Archived" } else { "Unarchived" };
    eprintln!("{action} repository {owner}/{repo}");
    Ok(())
}

async fn rename_repo(repo_args: &repo::RepoArgs, args: &RenameArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_edit()
        .owner(owner)
        .repo(repo)
        .body_map(|b| b.name(args.name.clone()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Renamed {owner}/{repo} → {owner}/{}", args.name);
    Ok(())
}

const DEPLOY_KEY_FIELDS: &[&str] = &[
    "id", "title", "key", "url", "read_only", "created_at",
];

async fn deploy_key(repo_args: &repo::RepoArgs, cmd: &DeployKeyCommand) -> Result<()> {
    match &cmd.action {
        DeployKeyAction::List(args) => deploy_key_list(repo_args, args).await,
        DeployKeyAction::Add(args) => deploy_key_add(repo_args, args).await,
        DeployKeyAction::Delete(args) => deploy_key_delete(repo_args, args).await,
    }
}

async fn deploy_key_list(repo_args: &repo::RepoArgs, args: &DeployKeyListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let keys = api
        .repo_list_keys()
        .owner(owner)
        .repo(repo)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &keys, DEPLOY_KEY_FIELDS);
    }

    if keys.is_empty() {
        eprintln!("No deploy keys found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<6} {:<30} {:<10} {}", "ID", "TITLE", "ACCESS", "FINGERPRINT");
    }

    for key in &keys {
        let id = key.id.unwrap_or(0);
        let title = key.title.as_deref().unwrap_or("");
        let access = if key.read_only.unwrap_or(true) {
            "read-only"
        } else {
            "read-write"
        };
        // Show abbreviated key fingerprint
        let key_str = key.key.as_deref().unwrap_or("");
        let fingerprint = if key_str.len() > 30 {
            format!("{}...{}", &key_str[..20], &key_str[key_str.len() - 10..])
        } else {
            key_str.to_string()
        };
        println!("{:<6} {:<30} {:<10} {}", id, title, access, fingerprint);
    }

    Ok(())
}

async fn deploy_key_add(repo_args: &repo::RepoArgs, args: &DeployKeyAddArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // If the key looks like a file path, read from file
    let key_content = if std::path::Path::new(&args.key).exists() {
        std::fs::read_to_string(&args.key)?
    } else {
        args.key.clone()
    };

    let key = api
        .repo_create_key()
        .owner(owner)
        .repo(repo)
        .body_map(|b| {
            b.title(args.title.clone())
                .key(key_content.trim().to_string())
                .read_only(args.read_only)
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let id = key.id.unwrap_or(0);
    eprintln!("Added deploy key '{}' (ID: {id})", args.title);
    Ok(())
}

async fn deploy_key_delete(repo_args: &repo::RepoArgs, args: &DeployKeyDeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_delete_key()
        .owner(owner)
        .repo(repo)
        .id(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Deleted deploy key #{}", args.id);
    Ok(())
}
